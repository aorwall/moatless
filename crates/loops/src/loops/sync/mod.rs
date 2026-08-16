//! Reconcile a repo's committed `.moatless/loops/` into the loop tables.
//!
//! A "loop" is an agent automation declared in git. Each `LOOP.md` maps to a
//! row in the existing `loops` / `loop_configs` / source tables with Git
//! configuration authority and stable repository provenance. Manual overrides
//! retain that provenance so reconciliation can skip them.
//!
//! Git owns configuration and nothing else. Execution is approved once in
//! Moatless, by an administrator naming a run-as user, and that approval then
//! survives every later re-sync — so a repository can keep its Loop
//! declarations current without an administrator re-approving each change.

use std::collections::HashSet;
use std::sync::Arc;

use crate::loops::models::LoopKind;
use chrono::{DateTime, Utc};
use moatless_api_schemas::loops::{LoopSyncSkip, LoopSyncSummary, LoopSyncWarning};
use tracing::{debug, warn};

mod error;
mod fetcher;
mod models;
mod parser;

use self::error::LoopError;
use self::fetcher::{GitHubLoopFetcher, LoopFetcher};
use self::models::{LoopFrontmatter, ParsedLoop};
pub use self::parser::parse_loop;
use crate::adapters::common::next_fire_at;
use crate::adapters::dao::AdapterConnectionDao;
use crate::adapters::models::RoutingMode;
use crate::loops::dao::{LoopDao, ManagedLoopUpsert, NewAdapterEventSource, NewScheduleSource};
use moatless_api_schemas::enums::RepositoryProvider as GitHostProvider;
use moatless_identity::auth::actor::Actor;
use moatless_identity::extractors::AuthUser;
use moatless_identity::git_host_repo::GitHostRepo;
use moatless_identity::git_host_tokens::GitHostTokens;
use moatless_tasks::tag::dao::TagDao;
use moatless_workspaces::repository::models::{Repository, RepositoryProvider};

/// Reconciles a repository's committed loop declarations into the loop tables.
///
/// Holds the four ports the reconcile tree reads: adapter connections, the loop
/// store, tags, and the git-host token seam.
pub struct LoopSyncService {
    adapter_connections: Arc<dyn AdapterConnectionDao>,
    loops: Arc<LoopDao>,
    tags: Arc<dyn TagDao>,
    git_host_tokens: Arc<GitHostTokens>,
}

impl LoopSyncService {
    pub fn new(
        adapter_connections: Arc<dyn AdapterConnectionDao>,
        loops: Arc<LoopDao>,
        tags: Arc<dyn TagDao>,
        git_host_tokens: Arc<GitHostTokens>,
    ) -> Self {
        Self {
            adapter_connections,
            loops,
            tags,
            git_host_tokens,
        }
    }
}

impl LoopSyncService {
    /// Fetch, parse, and reconcile the loops declared in a repository.
    ///
    /// GitHub-only for v1: non-GitHub repos return an empty summary with a warning.
    /// Returns the reconciliation summary; the fetch step can still fail hard
    /// (network / auth errors) which callers surface or swallow as appropriate.
    pub async fn sync_repo_loops(
        &self,
        repo: &Repository,
        auth: &AuthUser,
    ) -> Result<LoopSyncSummary, LoopError> {
        let Some(remote_url) = repo.remote_url.as_deref() else {
            return Ok(LoopSyncSummary::default());
        };
        let provider = repo.provider.unwrap_or(RepositoryProvider::Github);
        if provider != RepositoryProvider::Github {
            warn!(
                repository_id = %repo.id,
                provider = %provider.as_str(),
                "Loop sync only supports GitHub repos today — skipping"
            );
            return Ok(LoopSyncSummary::default());
        }

        let token = self.resolve_github_token(auth, remote_url).await;
        let fetcher = GitHubLoopFetcher::new(loop_http_client());
        let files = fetcher
            .fetch_loop_files(remote_url, token.as_deref(), repo.default_branch.as_deref())
            .await?;

        let present = files
            .iter()
            .map(|file| file.dir_name.clone())
            .collect::<HashSet<_>>();
        let mut parsed = Vec::new();
        let mut parse_skips = Vec::new();
        for file in files {
            match parse_loop(&file.content, &file.dir_name) {
                Ok(loop_) => parsed.push(loop_),
                Err(reason) => parse_skips.push(LoopSyncSkip {
                    loop_name: file.dir_name,
                    reason: format!("{reason}; existing Loop preserved"),
                }),
            }
        }

        self.sync_loops_snapshot(repo, auth, parsed, present, parse_skips)
            .await
    }

    /// Reset one overridden or deleted Loop to its current Git declaration.
    ///
    /// Unlike repository reconciliation this never considers sibling removals.
    /// The existing row keeps its identity and becomes Git-owned again; its
    /// execution approval is untouched, so a deleted Loop comes back awaiting
    /// approval and an overridden one keeps the run-as user it was given.
    pub async fn reset_repo_loop(
        &self,
        repo: &Repository,
        auth: &AuthUser,
        source_key: &str,
    ) -> Result<bool, LoopError> {
        let remote_url = repo.remote_url.as_deref().ok_or_else(|| LoopError::Fetch {
            git_url: repo.id.clone(),
            message: "repository has no remote URL".to_string(),
        })?;
        let provider = repo.provider.unwrap_or(RepositoryProvider::Github);
        if provider != RepositoryProvider::Github {
            return Err(LoopError::Fetch {
                git_url: remote_url.to_string(),
                message: "Loop sync only supports GitHub repositories".to_string(),
            });
        }
        let token = self.resolve_github_token(auth, remote_url).await;
        let fetcher = GitHubLoopFetcher::new(loop_http_client());
        let files = fetcher
            .fetch_loop_files(remote_url, token.as_deref(), repo.default_branch.as_deref())
            .await?;
        let file = files
            .into_iter()
            .find(|file| file.dir_name == source_key)
            .ok_or_else(|| LoopError::Fetch {
                git_url: remote_url.to_string(),
                message: format!("Loop directory `{source_key}` no longer exists"),
            })?;
        let parsed = parse_loop(&file.content, source_key)
            .map_err(|reason| LoopError::Internal(format!("Loop `{source_key}`: {reason}")))?;
        let resolved = match self.resolve_loop(repo, &parsed).await? {
            ResolveOutcome::Resolved(resolved) => resolved,
            ResolveOutcome::Skipped(reason) => return Err(LoopError::Internal(reason)),
        };
        let input = managed_loop_input(repo, auth, &parsed, &resolved);
        self.loops
            .reset_managed_loop(&input)
            .await
            .map_err(LoopError::from)
    }

    /// Reconcile a set of already-parsed loops into the loop tables.
    ///
    /// Reconciles by `(source_repository_id, source_key)` and removes Git-owned
    /// rows whose Loop directory is confirmed absent.
    pub async fn sync_loops(
        &self,
        repo: &Repository,
        auth: &AuthUser,
        loops: Vec<ParsedLoop>,
    ) -> Result<LoopSyncSummary, LoopError> {
        let present = loops
            .iter()
            .map(|loop_| loop_.dir_name.clone())
            .collect::<HashSet<_>>();
        self.sync_loops_snapshot(repo, auth, loops, present, Vec::new())
            .await
    }

    async fn sync_loops_snapshot(
        &self,
        repo: &Repository,
        auth: &AuthUser,
        loops: Vec<ParsedLoop>,
        present: HashSet<String>,
        parse_skips: Vec<LoopSyncSkip>,
    ) -> Result<LoopSyncSummary, LoopError> {
        let mut summary = LoopSyncSummary {
            skipped: parse_skips,
            ..LoopSyncSummary::default()
        };
        let mut desired = Vec::new();

        for parsed in &loops {
            if parsed.frontmatter.active.is_some() {
                summary.warnings.push(LoopSyncWarning {
                    loop_name: parsed.dir_name.clone(),
                    field: "active".to_string(),
                    message: "ignored: Git cannot approve Loop execution".to_string(),
                });
            }
            if parsed.frontmatter.run_as_user_id.is_some() {
                summary.warnings.push(LoopSyncWarning {
                    loop_name: parsed.dir_name.clone(),
                    field: "runAsUserId".to_string(),
                    message: "ignored: select the run-as user during administrator activation"
                        .to_string(),
                });
            }
            match self.resolve_loop(repo, parsed).await? {
                ResolveOutcome::Skipped(reason) => {
                    debug!(repository_id = %repo.id, loop = %parsed.dir_name, %reason, "Skipping loop");
                    summary.skipped.push(LoopSyncSkip {
                        loop_name: parsed.dir_name.clone(),
                        reason,
                    });
                }
                ResolveOutcome::Resolved(resolved) => {
                    desired.push(managed_loop_input(repo, auth, parsed, &resolved));
                }
            }
        }

        let reconciled = self
            .loops
            .reconcile_managed_loops(&repo.id, &desired, &present)
            .await?;
        summary.created = reconciled.created;
        summary.updated = reconciled.updated;
        summary.removed = reconciled.removed;
        summary.overrides_skipped = reconciled.overrides_skipped;
        summary.skipped.extend(
        reconciled
            .skipped_overrides
            .into_iter()
            .map(|skip| LoopSyncSkip {
                loop_name: skip.source_key,
                reason: if skip.deleted {
                    "Git-origin Loop has a local deletion tombstone; reset it explicitly to restore"
                        .to_string()
                } else {
                    "Git-origin Loop has a local override; reset it explicitly to resume Git sync"
                        .to_string()
                },
            }),
    );

        Ok(summary)
    }
}

/// Resolved values for one loop, ready to write to the loop tables.
struct ResolvedLoop {
    kind: LoopKind,
    /// Display name for `loops.name`, from the required `name` frontmatter.
    name: String,
    routing_mode: RoutingMode,
    agent_type: String,
    prompt: Option<String>,
    tag_ids: Vec<String>,
    skills: Vec<String>,
    inactive_ttl_days: Option<i32>,
    adapter: Option<ResolvedAdapterSource>,
    schedule: Option<ResolvedScheduleSource>,
}

struct ResolvedAdapterSource {
    adapter_kind: String,
    connection_id: String,
    source_matcher: String,
    event_filter: serde_json::Value,
    adapter_config: Option<serde_json::Value>,
}

struct ResolvedScheduleSource {
    cron_expression: String,
    timezone: String,
    message_template: String,
    next_fire_at: DateTime<Utc>,
}

enum ResolveOutcome {
    Resolved(Box<ResolvedLoop>),
    Skipped(String),
}

impl LoopSyncService {
    /// Resolve a parsed loop into concrete persistence values, or a skip reason.
    ///
    /// Hard errors (DB lookups) propagate; logical problems (no connection,
    /// invalid cron, unknown `on`) become a `Skipped` outcome.
    async fn resolve_loop(
        &self,
        repo: &Repository,
        parsed: &ParsedLoop,
    ) -> Result<ResolveOutcome, LoopError> {
        let fm = &parsed.frontmatter;

        let routing_mode = match fm.routing_mode.as_deref() {
            None => RoutingMode::BySubject,
            Some(s) => match RoutingMode::try_from(s) {
                Ok(rm) => rm,
                Err(e) => return Ok(ResolveOutcome::Skipped(format!("invalid routingMode: {e}"))),
            },
        };

        let agent_type = fm
            .agent_type
            .clone()
            .unwrap_or_else(|| "claude-code".to_string());
        let prompt = if parsed.prompt_body.is_empty() {
            None
        } else {
            Some(parsed.prompt_body.clone())
        };

        // Resolve tag names → ids, creating any that don't exist yet.
        let tag_ids: Vec<String> = match fm.tags.as_deref() {
            Some(names) if !names.is_empty() => self
                .tags
                .find_or_create_tags(names)
                .await?
                .into_iter()
                .map(|t| t.tag_id)
                .collect(),
            _ => Vec::new(),
        };

        // Skill names are not resolved here: a skill exists on the pod that runs
        // the task, not in this database.
        let skills = fm.skills.clone().unwrap_or_default();

        let (kind, adapter, schedule) = match fm.on.trim() {
            "github_pr" => {
                let Some(remote_url) = repo.remote_url.as_deref() else {
                    return Ok(ResolveOutcome::Skipped(
                        "repository has no remote URL".to_string(),
                    ));
                };
                let Some(parsed) = GitHostRepo::parse(GitHostProvider::Github, remote_url) else {
                    return Ok(ResolveOutcome::Skipped(format!(
                        "cannot parse owner/repo from {remote_url}"
                    )));
                };
                let (owner, repo_name) = (parsed.owner(), parsed.name());
                let Some(connection) = self
                    .adapter_connections
                    .find_by_external_account("github", owner)
                    .await?
                else {
                    return Ok(ResolveOutcome::Skipped(format!(
                        "no github connection for org `{owner}` — set up the GitHub App + connection first"
                    )));
                };
                let source_matcher = fm
                    .source_matcher
                    .clone()
                    .unwrap_or_else(|| format!("{owner}/{repo_name}"));
                let event_filter = fm
                    .event_filter
                    .clone()
                    .unwrap_or_else(|| serde_json::json!([]));
                (
                    LoopKind::AdapterEvent,
                    Some(ResolvedAdapterSource {
                        adapter_kind: "github".to_string(),
                        connection_id: connection.id,
                        source_matcher,
                        event_filter,
                        adapter_config: fm.adapter_config.clone(),
                    }),
                    None,
                )
            }
            "slack" => {
                // Resolve the slack connection from `teamId`, or fall back to the
                // single configured slack connection when unambiguous.
                let connection = if let Some(team) = fm.team_id.as_deref() {
                    let Some(connection) = self
                        .adapter_connections
                        .find_by_external_account("slack", team)
                        .await?
                    else {
                        return Ok(ResolveOutcome::Skipped(format!(
                            "no slack connection for team `{team}`"
                        )));
                    };
                    connection
                } else {
                    let conns = self.adapter_connections.list_by_provider("slack").await?;
                    match conns.as_slice() {
                        [one] => one.clone(),
                        [] => {
                            return Ok(ResolveOutcome::Skipped(
                                "no slack connection configured".to_string(),
                            ));
                        }
                        _ => {
                            return Ok(ResolveOutcome::Skipped(
                                "multiple slack connections — set teamId".to_string(),
                            ));
                        }
                    }
                };
                // Unlike github there is no `owner/repo` default — the channel id is required.
                let Some(source_matcher) = fm.source_matcher.clone() else {
                    return Ok(ResolveOutcome::Skipped(
                        "slack loop missing sourceMatcher (channel id)".to_string(),
                    ));
                };
                let event_filter = fm
                    .event_filter
                    .clone()
                    .unwrap_or_else(|| serde_json::json!([]));
                (
                    LoopKind::AdapterEvent,
                    Some(ResolvedAdapterSource {
                        adapter_kind: "slack".to_string(),
                        connection_id: connection.id,
                        source_matcher,
                        event_filter,
                        adapter_config: fm.adapter_config.clone(),
                    }),
                    None,
                )
            }
            "linear" => {
                // Resolve the Linear connection from `teamId` (the Linear
                // workspace/organization id), or fall back to the single configured
                // Linear connection when unambiguous.
                let connection = if let Some(org) = fm.team_id.as_deref() {
                    let Some(connection) = self
                        .adapter_connections
                        .find_by_external_account("linear", org)
                        .await?
                    else {
                        return Ok(ResolveOutcome::Skipped(format!(
                            "no linear connection for workspace `{org}`"
                        )));
                    };
                    connection
                } else {
                    let conns = self.adapter_connections.list_by_provider("linear").await?;
                    match conns.as_slice() {
                        [one] => one.clone(),
                        [] => {
                            return Ok(ResolveOutcome::Skipped(
                                "no linear connection configured".to_string(),
                            ));
                        }
                        _ => {
                            return Ok(ResolveOutcome::Skipped(
                                "multiple linear connections — set teamId (workspace id)"
                                    .to_string(),
                            ));
                        }
                    }
                };
                // The listen scope is the Linear TEAM uuid — required, no
                // `owner/repo` default (issue events are routed by team).
                let Some(source_matcher) = fm.source_matcher.clone() else {
                    return Ok(ResolveOutcome::Skipped(
                        "linear loop missing sourceMatcher (team id)".to_string(),
                    ));
                };
                let event_filter = fm
                    .event_filter
                    .clone()
                    .unwrap_or_else(|| serde_json::json!([]));
                (
                    LoopKind::AdapterEvent,
                    Some(ResolvedAdapterSource {
                        adapter_kind: "linear".to_string(),
                        connection_id: connection.id,
                        source_matcher,
                        event_filter,
                        adapter_config: Some(build_linear_adapter_config(fm)),
                    }),
                    None,
                )
            }
            "schedule" => {
                let sched = fm.schedule.as_ref();
                let Some(cron) = sched
                    .and_then(|s| s.cron.clone())
                    .filter(|c| !c.trim().is_empty())
                else {
                    return Ok(ResolveOutcome::Skipped(
                        "schedule loop missing schedule.cron".to_string(),
                    ));
                };
                let next_fire = match next_fire_at(&cron) {
                    Ok(t) => t,
                    Err(e) => return Ok(ResolveOutcome::Skipped(format!("invalid cron: {e}"))),
                };
                let timezone = sched
                    .and_then(|s| s.timezone.clone())
                    .unwrap_or_else(|| "UTC".to_string());
                let message_template = sched
                    .and_then(|s| s.message.clone())
                    .unwrap_or_else(|| default_schedule_message(&fm.name));
                (
                    LoopKind::Schedule,
                    None,
                    Some(ResolvedScheduleSource {
                        cron_expression: cron,
                        timezone,
                        message_template,
                        next_fire_at: next_fire,
                    }),
                )
            }
            other => {
                return Ok(ResolveOutcome::Skipped(format!(
                    "unknown `on` value: {other}"
                )));
            }
        };

        Ok(ResolveOutcome::Resolved(Box::new(ResolvedLoop {
            kind,
            name: fm.name.clone(),
            routing_mode,
            agent_type,
            prompt,
            tag_ids,
            skills,
            inactive_ttl_days: fm.inactive_ttl_days,
            adapter,
            schedule,
        })))
    }
}

/// Convert one resolved Git declaration into the store's reconciliation input.
fn managed_loop_input(
    repo: &Repository,
    auth: &AuthUser,
    parsed: &ParsedLoop,
    resolved: &ResolvedLoop,
) -> ManagedLoopUpsert {
    let created_by = repo
        .created_by
        .clone()
        .unwrap_or_else(|| auth.user_id.to_string());

    let adapter = resolved.adapter.as_ref().map(|a| NewAdapterEventSource {
        adapter_kind: a.adapter_kind.clone(),
        connection_id: a.connection_id.clone(),
        source_matcher: a.source_matcher.clone(),
        source_name: None,
        event_filter: a.event_filter.clone(),
        adapter_config: a.adapter_config.clone(),
        allow_direct_send: false,
        close_task_on_close: true,
        notify_on_success: false,
    });
    let schedule = resolved.schedule.as_ref().map(|s| NewScheduleSource {
        cron_expression: s.cron_expression.clone(),
        timezone: s.timezone.clone(),
        message_template: s.message_template.clone(),
        next_fire_at: s.next_fire_at,
    });

    ManagedLoopUpsert {
        source_repository_id: repo.id.clone(),
        source_key: parsed.dir_name.clone(),
        source_config_path: format!(".moatless/loops/{}/LOOP.md", parsed.dir_name),
        trigger_name: resolved.name.clone(),
        kind: resolved.kind.as_str().to_string(),
        scope: repo.scope,
        owner_user_id: repo.owner_user_id.clone(),
        created_by,
        routing_mode: resolved.routing_mode,
        prompt: resolved.prompt.clone(),
        tag_ids: resolved.tag_ids.clone(),
        skills: resolved.skills.clone(),
        agent_type: resolved.agent_type.clone(),
        inactive_ttl_days: resolved.inactive_ttl_days,
        adapter,
        schedule,
    }
}

impl LoopSyncService {
    /// The caller's own GitHub token for loop fetching, best-effort.
    ///
    /// Returns `None` — an unauthenticated fetch, which works for public
    /// repositories — when they have not connected GitHub or the credential
    /// cannot be minted.
    async fn resolve_github_token(&self, auth: &AuthUser, remote_url: &str) -> Option<String> {
        let repo = GitHostRepo::parse(GitHostProvider::Github, remote_url)?;
        match self
            .git_host_tokens
            .token(&Actor::caller(&auth.scope_context()), &repo)
            .await
        {
            Ok(token) => Some(token),
            Err(e) => {
                warn!(error = %e, "Loop sync: fetching unauthenticated");
                None
            }
        }
    }
}

fn loop_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("moatless-vibe-backend")
        .build()
        .unwrap_or_else(|e| {
            warn!(error = %e, "Loop sync: failed to build reqwest client, using default");
            reqwest::Client::new()
        })
}

// =========================================================================
// Pure helpers (unit-tested below)
// =========================================================================

/// Default kickoff message for a schedule loop with no explicit `schedule.message`.
pub(crate) fn default_schedule_message(loop_name: &str) -> String {
    format!("Run the `{loop_name}` loop.")
}

/// Build the `adapter_config` (stored as `adapter_config`) for an `on: linear`
/// loop: start from any user-provided `adapterConfig` object, layer the loop's
/// first-class status/project/label knobs on top, and default
/// `create_on_status_entry` on unless the user set it — the original Linear loop
/// use case is spawning tasks on status entry. A loop can opt out
/// (`createOnStatusEntry: false`) to run mention-mode only.
pub(crate) fn build_linear_adapter_config(fm: &LoopFrontmatter) -> serde_json::Value {
    let mut obj = match fm.adapter_config.clone() {
        Some(serde_json::Value::Object(m)) => m,
        _ => serde_json::Map::new(),
    };
    if let Some(create) = fm.create_on_status_entry {
        // First-class field wins (consistent with trigger_states/project_ids).
        obj.insert(
            "create_on_status_entry".to_string(),
            serde_json::Value::Bool(create),
        );
    } else {
        // Preserve the `adapterConfig` escape hatch and the default-on behavior.
        obj.entry("create_on_status_entry".to_string())
            .or_insert(serde_json::Value::Bool(true));
    }
    if let Some(states) = fm.trigger_states.as_ref() {
        obj.insert("trigger_states".to_string(), serde_json::json!(states));
    }
    if let Some(projects) = fm.project_ids.as_ref() {
        obj.insert("project_ids".to_string(), serde_json::json!(projects));
    }
    if let Some(labels) = fm.require_labels.as_ref() {
        obj.insert("require_labels".to_string(), serde_json::json!(labels));
    }
    if let Some(start) = fm.start_state.as_ref() {
        obj.insert("start_state".to_string(), serde_json::json!(start));
    }
    serde_json::Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_schedule_message_uses_loop_name() {
        assert_eq!(
            default_schedule_message("daily-health-check"),
            "Run the `daily-health-check` loop."
        );
    }

    fn linear_fm(
        trigger_states: Option<Vec<String>>,
        project_ids: Option<Vec<String>>,
        adapter_config: Option<serde_json::Value>,
    ) -> LoopFrontmatter {
        LoopFrontmatter {
            on: "linear".to_string(),
            name: "Ready Pool".to_string(),
            agent_type: None,
            routing_mode: None,
            tags: None,
            skills: None,
            run_as_user_id: None,
            source_matcher: Some("team-uuid".to_string()),
            team_id: None,
            trigger_states,
            project_ids,
            require_labels: None,
            start_state: Some("In Progress".to_string()),
            create_on_status_entry: None,
            event_filter: None,
            adapter_config,
            active: None,
            inactive_ttl_days: None,
            schedule: None,
        }
    }

    #[test]
    fn linear_config_defaults_to_create_on_status_entry() {
        let cfg = build_linear_adapter_config(&linear_fm(
            Some(vec!["Ready".to_string()]),
            Some(vec!["proj-1".to_string()]),
            None,
        ));
        assert_eq!(cfg["create_on_status_entry"], serde_json::json!(true));
        assert_eq!(cfg["trigger_states"], serde_json::json!(["Ready"]));
        assert_eq!(cfg["project_ids"], serde_json::json!(["proj-1"]));
        assert_eq!(cfg["start_state"], serde_json::json!("In Progress"));
    }

    #[test]
    fn linear_config_respects_explicit_opt_out() {
        // A user who sets create_on_status_entry:false in adapterConfig keeps it.
        let cfg = build_linear_adapter_config(&linear_fm(
            None,
            None,
            Some(serde_json::json!({ "create_on_status_entry": false })),
        ));
        assert_eq!(cfg["create_on_status_entry"], serde_json::json!(false));
    }

    #[test]
    fn linear_config_frontmatter_overrides_adapter_config_states() {
        // First-class triggerStates wins over a value nested in adapterConfig.
        let cfg = build_linear_adapter_config(&linear_fm(
            Some(vec!["Ready".to_string()]),
            None,
            Some(serde_json::json!({ "trigger_states": ["Todo"], "require_labels": ["x"] })),
        ));
        assert_eq!(cfg["trigger_states"], serde_json::json!(["Ready"]));
        // Untouched adapterConfig keys are preserved.
        assert_eq!(cfg["require_labels"], serde_json::json!(["x"]));
    }

    #[test]
    fn linear_config_create_on_status_entry_first_class_false() {
        // createOnStatusEntry: false authors a mention-mode loop (status-entry off).
        let mut fm = linear_fm(Some(vec!["Ready".to_string()]), None, None);
        fm.create_on_status_entry = Some(false);
        let cfg = build_linear_adapter_config(&fm);
        assert_eq!(cfg["create_on_status_entry"], serde_json::json!(false));
    }

    #[test]
    fn linear_config_first_class_create_overrides_adapter_config() {
        // First-class createOnStatusEntry wins over a value nested in adapterConfig.
        let mut fm = linear_fm(
            None,
            None,
            Some(serde_json::json!({ "create_on_status_entry": true })),
        );
        fm.create_on_status_entry = Some(false);
        let cfg = build_linear_adapter_config(&fm);
        assert_eq!(cfg["create_on_status_entry"], serde_json::json!(false));
    }
}
