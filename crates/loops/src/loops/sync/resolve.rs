//! Resolve parsed Loop declarations into persistence inputs.

use super::error::LoopError;
use super::models::{LoopFrontmatter, ParsedLoop};
use super::reconcile::LoopSyncService;
use crate::loops::models::{ManagedLoopUpsert, NewAdapterEventSource, NewScheduleSource};
use crate::loops::schedule::next_fire_at;
use chrono::{DateTime, Utc};
use moatless_tasks::agent::catalog;
use moatless_types::access_control::{Actor, ScopeContext};
use moatless_types::agent::AgentProfile;
use moatless_types::loops::{LoopKind, RoutingMode};
use moatless_workspace_types::repository::{
    Repository, RepositoryProvider as GitHostProvider, RepositoryRemote,
};
use tracing::warn;

/// Resolved values for one loop, ready to write to the loop tables.
pub(super) struct ResolvedLoop {
    kind: LoopKind,
    /// Display name for `loops.name`, from the required `name` frontmatter.
    name: String,
    routing_mode: RoutingMode,
    /// See [`moatless_types::loops::LoopContext::workspace_id`].
    workspace_id: String,
    agent_profile: AgentProfile,
    model: Option<String>,
    prompt: Option<String>,
    tag_ids: Vec<String>,
    skills: Vec<String>,
    inactive_ttl_days: Option<i32>,
    stop_when_done: Option<bool>,
    message_buffer_seconds: Option<i32>,
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

pub(super) enum ResolveOutcome {
    Resolved(Box<ResolvedLoop>),
    Skipped(String),
}

/// The agent a Loop declaration runs on: its `agentProfile`, else the default.
pub(super) fn declared_agent_profile(fm: &LoopFrontmatter) -> Result<AgentProfile, String> {
    fm.agent_profile
        .as_deref()
        .map_or(Ok(AgentProfile::ClaudeCode), str::parse)
}

impl LoopSyncService {
    /// Resolve a parsed loop into concrete persistence values, or a skip
    /// reason.
    ///
    /// Hard errors (DB lookups) propagate; logical problems (no connection,
    /// invalid cron, unknown `on`) become a `Skipped` outcome.
    pub(super) async fn resolve_loop(
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

        // A named Workspace must be one this repository is placed in: firing into
        // any other would run the Loop against a checkout it never declared. An
        // unnamed one resolves to the repository's own Workspace, primary
        // placement first, because every Loop stores a concrete Workspace.
        let placed = self.workspaces.for_repository(&repo.id).await?;
        let workspace_id = match fm.workspace.as_deref().map(str::trim) {
            None | Some("") => {
                let Some(workspace) = placed.into_iter().next() else {
                    return Ok(ResolveOutcome::Skipped(
                        "no workspace holds this repository — declare `workspace:` or place it in one"
                            .to_string(),
                    ));
                };
                workspace.id
            }
            Some(name) => {
                let Some(workspace) = placed.into_iter().find(|w| w.name == name) else {
                    return Ok(ResolveOutcome::Skipped(format!(
                        "no workspace named `{name}` holds this repository"
                    )));
                };
                workspace.id
            }
        };

        let agent_profile = match declared_agent_profile(fm) {
            Ok(profile) => profile,
            Err(e) => return Ok(ResolveOutcome::Skipped(e)),
        };
        let model = fm.model.clone();
        if let Err(e) = catalog::validate_agent_model(agent_profile, model.as_deref()) {
            return Ok(ResolveOutcome::Skipped(e));
        }
        let prompt = if parsed.prompt_body.is_empty() {
            None
        } else {
            Some(parsed.prompt_body.clone())
        };

        // Resolve tag names → ids, creating any that don't exist yet.
        let tag_ids: Vec<String> = match fm.tags.as_deref() {
            Some(names) if !names.is_empty() => self.tags.resolve(names).await?,
            _ => Vec::new(),
        };

        // Skill names are not resolved here: a skill exists on the pod that runs
        // the task, not in this database.
        let skills = fm.skills.clone().unwrap_or_default();

        let (kind, adapter, schedule) = match fm.on.trim() {
            "github_pr" => {
                let Some(remote) = repo.remote.as_ref() else {
                    return Ok(ResolveOutcome::Skipped(
                        "repository has no remote URL".to_string(),
                    ));
                };
                let Some(parsed) = remote
                    .host_repo
                    .as_ref()
                    .filter(|repo| repo.provider() == GitHostProvider::Github)
                else {
                    return Ok(ResolveOutcome::Skipped(format!(
                        "cannot resolve GitHub repository from {}",
                        remote.url
                    )));
                };
                let (owner, repo_name) = (parsed.owner(), parsed.name());
                let Some(connection) = self
                    .adapter_connections
                    .by_external_account("github", owner)
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
                        .by_external_account("slack", team)
                        .await?
                    else {
                        return Ok(ResolveOutcome::Skipped(format!(
                            "no slack connection for team `{team}`"
                        )));
                    };
                    connection
                } else {
                    let conns = self.adapter_connections.by_provider("slack").await?;
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
                        .by_external_account("linear", org)
                        .await?
                    else {
                        return Ok(ResolveOutcome::Skipped(format!(
                            "no linear connection for workspace `{org}`"
                        )));
                    };
                    connection
                } else {
                    let conns = self.adapter_connections.by_provider("linear").await?;
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
                    Err(e) => {
                        return Ok(ResolveOutcome::Skipped(format!(
                            "invalid cron: Invalid request: {e}"
                        )));
                    }
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
            workspace_id,
            agent_profile,
            model,
            prompt,
            tag_ids,
            skills,
            inactive_ttl_days: fm.inactive_ttl_days,
            stop_when_done: fm.stop_when_done,
            message_buffer_seconds: fm.message_buffer_seconds,
            adapter,
            schedule,
        })))
    }
}

/// Convert one resolved Git declaration into the store's reconciliation input.
pub(super) fn managed_loop_input(
    repo: &Repository,
    ctx: &ScopeContext,
    parsed: &ParsedLoop,
    resolved: &ResolvedLoop,
) -> ManagedLoopUpsert {
    let created_by = repo
        .created_by
        .clone()
        .unwrap_or_else(|| ctx.user_id.clone());

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
        workspace_id: resolved.workspace_id.clone(),
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
        agent_profile: resolved.agent_profile,
        model: resolved.model.clone(),
        inactive_ttl_days: resolved.inactive_ttl_days,
        stop_when_done: resolved.stop_when_done,
        message_buffer_seconds: resolved.message_buffer_seconds,
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
    pub(super) async fn resolve_github_token(
        &self,
        ctx: &ScopeContext,
        remote: &RepositoryRemote,
    ) -> Option<String> {
        let repo = remote
            .host_repo
            .as_ref()
            .filter(|repo| repo.provider() == GitHostProvider::Github)?;
        match self
            .git_host_tokens
            .token(&Actor::caller(ctx), &repo.credential_account())
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

// =========================================================================
// Pure helpers (unit-tested below)
// =========================================================================

/// Default kickoff message for a schedule loop with no explicit
/// `schedule.message`.
pub(crate) fn default_schedule_message(loop_name: &str) -> String {
    format!("Run the `{loop_name}` loop.")
}

/// Build `adapter_config` for an `on: linear` loop.
///
/// Merges the user's `adapterConfig` object with the loop's first-class
/// status/project/label knobs, defaulting `create_on_status_entry` on unless
/// set (opt out via `createOnStatusEntry: false` for mention-mode only).
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
            workspace: None,
            agent_profile: None,
            model: None,
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
            stop_when_done: None,
            message_buffer_seconds: None,
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
