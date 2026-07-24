//! Reconcile a repo's committed `.moatless/loops/` into the trigger tables.
//!
//! A "loop" is an agent automation declared in git. Each `LOOP.md` maps to a
//! row in the existing `triggers` / `trigger_rules` / `*_trigger_sources`
//! tables, tagged `managed_by = 'repo_loop'` and keyed by
//! `(source_repository_id, loop_name)` so reconciliation never touches
//! manually-created triggers.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use moatless_api_schemas::loops::{LoopSyncSkip, LoopSyncSummary};
use moatless_api_schemas::triggers::TriggerKind;
use tracing::{debug, warn};

use super::error::LoopError;
use super::fetcher::{GitHubLoopFetcher, LoopFetcher};
use super::models::{LoopFrontmatter, ParsedLoop};
use super::parser::parse_loop;
use crate::adapters::common::next_fire_at;
use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::models::RoutingMode;
use crate::extractors::AuthUser;
use crate::repository::access::{parse_github_owner_repo, resolve_repository_github_token};
use crate::repository::models::{Repository, RepositoryProvider};
use crate::state::AppState;

/// Fetch, parse, and reconcile the loops declared in a repository.
///
/// GitHub-only for v1: non-GitHub repos return an empty summary with a warning.
/// Returns the reconciliation summary; the fetch step can still fail hard
/// (network / auth errors) which callers surface or swallow as appropriate.
pub async fn sync_repo_loops(
    state: &AppState,
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

    let token = resolve_github_token(state, repo, remote_url).await;
    let fetcher = GitHubLoopFetcher::new(loop_http_client());
    let files = fetcher
        .fetch_loop_files(remote_url, token.as_deref())
        .await?;

    let parsed: Vec<ParsedLoop> = files
        .iter()
        .filter_map(|f| parse_loop(&f.content, &f.dir_name))
        .collect();

    sync_loops(state, repo, auth, parsed).await
}

/// Reconcile a set of already-parsed loops into the trigger tables.
///
/// Upserts each loop by `(source_repository_id, loop_name)` and deletes any
/// `repo_loop` trigger for this repo whose loop directory is no longer present.
pub async fn sync_loops(
    state: &AppState,
    repo: &Repository,
    auth: &AuthUser,
    loops: Vec<ParsedLoop>,
) -> Result<LoopSyncSummary, LoopError> {
    let mut summary = LoopSyncSummary::default();
    // Every loop that exists in the tree (even if skipped) — so we never delete
    // a previously-synced trigger just because it couldn't sync this round.
    let mut present: HashSet<String> = HashSet::new();

    for parsed in &loops {
        present.insert(parsed.name.clone());
        match resolve_loop(state, repo, parsed).await? {
            ResolveOutcome::Skipped(reason) => {
                debug!(repository_id = %repo.id, loop = %parsed.name, %reason, "Skipping loop");
                summary.skipped.push(LoopSyncSkip {
                    loop_name: parsed.name.clone(),
                    reason,
                });
            }
            ResolveOutcome::Resolved(resolved) => {
                let created = upsert_loop(state, repo, auth, parsed, &resolved).await?;
                if created {
                    summary.created += 1;
                } else {
                    summary.updated += 1;
                }
            }
        }
    }

    // Reconcile deletes: drop repo_loop triggers whose directory is gone.
    let existing: Vec<(String, String)> = sqlx::query_as(
        "SELECT id, loop_name FROM triggers \
         WHERE source_repository_id = $1 AND managed_by = 'repo_loop' AND loop_name IS NOT NULL",
    )
    .bind(&repo.id)
    .fetch_all(&state.db)
    .await?;

    let to_delete = names_to_delete(&existing, &present);
    for id in &to_delete {
        // Cascade drops trigger_rules + source rows.
        sqlx::query("DELETE FROM triggers WHERE id = $1")
            .bind(id)
            .execute(&state.db)
            .await?;
    }
    summary.removed = to_delete.len() as u32;

    Ok(summary)
}

/// Resolved values for one loop, ready to write to the trigger tables.
struct ResolvedLoop {
    kind: TriggerKind,
    /// Display name override for `triggers.name` (defaults to the loop dir name).
    name: Option<String>,
    active: bool,
    routing_mode: RoutingMode,
    agent_type: String,
    prompt: Option<String>,
    tag_ids: Vec<String>,
    run_as_user_id: Option<String>,
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

/// Resolve a parsed loop into concrete trigger values, or a skip reason.
///
/// Hard errors (DB lookups) propagate; logical problems (no connection,
/// invalid cron, unknown `on`) become a `Skipped` outcome.
async fn resolve_loop(
    state: &AppState,
    repo: &Repository,
    parsed: &ParsedLoop,
) -> Result<ResolveOutcome, LoopError> {
    let fm = &parsed.frontmatter;

    let run_as_user_id = resolve_run_as_user_id(
        fm.run_as_user_id.as_deref(),
        state.config.default_run_as_user_id.as_deref(),
        repo.owner_user_id.as_deref(),
        repo.created_by.as_deref(),
    );
    // A committed `runAsUserId` (or the configured default) may be a legacy
    // provider id; resolve it to the internal uuid, falling back to the repo
    // owner/creator (already uuids) when it names no known user.
    let run_as_user_id = match run_as_user_id {
        Some(raw) => resolve_run_as_to_internal_id(state.users.as_ref(), &raw)
            .await
            .or_else(|| {
                warn!(run_as = %raw, "runAsUserId does not resolve to a known user; falling back to repo owner/creator");
                repo.owner_user_id.clone().or_else(|| repo.created_by.clone())
            }),
        None => None,
    };

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
    let active = fm.active.unwrap_or(true);
    let prompt = if parsed.prompt_body.is_empty() {
        None
    } else {
        Some(parsed.prompt_body.clone())
    };

    // Resolve tag names → ids, creating any that don't exist yet.
    let tag_ids: Vec<String> = match fm.tags.as_deref() {
        Some(names) if !names.is_empty() => state
            .tags
            .find_or_create_tags(names)
            .await?
            .into_iter()
            .map(|t| t.tag_id)
            .collect(),
        _ => Vec::new(),
    };

    let (kind, adapter, schedule) = match fm.on.trim() {
        "github_pr" => {
            let Some(remote_url) = repo.remote_url.as_deref() else {
                return Ok(ResolveOutcome::Skipped(
                    "repository has no remote URL".to_string(),
                ));
            };
            let Some((owner, repo_name)) = parse_github_owner_repo(remote_url) else {
                return Ok(ResolveOutcome::Skipped(format!(
                    "cannot parse owner/repo from {remote_url}"
                )));
            };
            let Some(connection) = PgAdapterConnectionRepository(state.db.clone())
                .find_by_external_account("github", &owner)
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
                TriggerKind::AdapterEvent,
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
                let Some(connection) = PgAdapterConnectionRepository(state.db.clone())
                    .find_by_external_account("slack", team)
                    .await?
                else {
                    return Ok(ResolveOutcome::Skipped(format!(
                        "no slack connection for team `{team}`"
                    )));
                };
                connection
            } else {
                let conns = PgAdapterConnectionRepository(state.db.clone())
                    .list_by_provider("slack")
                    .await?;
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
                TriggerKind::AdapterEvent,
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
                let Some(connection) = PgAdapterConnectionRepository(state.db.clone())
                    .find_by_external_account("linear", org)
                    .await?
                else {
                    return Ok(ResolveOutcome::Skipped(format!(
                        "no linear connection for workspace `{org}`"
                    )));
                };
                connection
            } else {
                let conns = PgAdapterConnectionRepository(state.db.clone())
                    .list_by_provider("linear")
                    .await?;
                match conns.as_slice() {
                    [one] => one.clone(),
                    [] => {
                        return Ok(ResolveOutcome::Skipped(
                            "no linear connection configured".to_string(),
                        ));
                    }
                    _ => {
                        return Ok(ResolveOutcome::Skipped(
                            "multiple linear connections — set teamId (workspace id)".to_string(),
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
                TriggerKind::AdapterEvent,
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
                .unwrap_or_else(|| default_schedule_message(&parsed.name));
            (
                TriggerKind::Schedule,
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
        active,
        routing_mode,
        agent_type,
        prompt,
        tag_ids,
        run_as_user_id,
        inactive_ttl_days: fm.inactive_ttl_days,
        adapter,
        schedule,
    })))
}

/// Insert or update the trigger rows for one resolved loop. Returns `true` when
/// a fresh trigger was inserted, `false` when an existing one was updated.
async fn upsert_loop(
    state: &AppState,
    repo: &Repository,
    auth: &AuthUser,
    parsed: &ParsedLoop,
    resolved: &ResolvedLoop,
) -> Result<bool, LoopError> {
    let now = Utc::now();
    let created_by = repo
        .created_by
        .clone()
        .unwrap_or_else(|| auth.user_id.to_string());
    let trigger_name = resolved.name.clone().or_else(|| Some(parsed.name.clone()));

    let mut tx = state.db.begin().await?;

    let existing_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM triggers \
         WHERE source_repository_id = $1 AND loop_name = $2 AND managed_by = 'repo_loop'",
    )
    .bind(&repo.id)
    .bind(&parsed.name)
    .fetch_optional(&mut *tx)
    .await?;

    let (trigger_id, created) = if let Some(id) = existing_id {
        sqlx::query(
            "UPDATE triggers \
             SET name = $2, kind = $3, active = $4, scope = $5, team_id = $6, \
                 owner_user_id = $7, updated_at = $8 \
             WHERE id = $1",
        )
        .bind(&id)
        .bind(&trigger_name)
        .bind(resolved.kind.as_str())
        .bind(resolved.active)
        .bind(repo.scope)
        .bind(&repo.team_id)
        .bind(&repo.owner_user_id)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        (id, false)
    } else {
        let id = format!("tr_{}", uuid::Uuid::new_v4());
        sqlx::query(
            "INSERT INTO triggers \
             (id, name, kind, active, scope, team_id, owner_user_id, created_by, \
              managed_by, source_repository_id, loop_name, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'repo_loop', $9, $10, $11, $11)",
        )
        .bind(&id)
        .bind(&trigger_name)
        .bind(resolved.kind.as_str())
        .bind(resolved.active)
        .bind(repo.scope)
        .bind(&repo.team_id)
        .bind(&repo.owner_user_id)
        .bind(&created_by)
        .bind(&repo.id)
        .bind(&parsed.name)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        (id, true)
    };

    // Upsert the rule row (handles both insert + update paths).
    sqlx::query(
        "INSERT INTO trigger_rules \
         (trigger_id, repository_id, routing_mode, ongoing_task_id, prompt, task_name, \
          tag_ids, run_as_user_id, agent_type, inactive_ttl_days) \
         VALUES ($1, $2, $3, NULL, $4, NULL, $5, $6, $7, $8) \
         ON CONFLICT (trigger_id) DO UPDATE SET \
           repository_id = EXCLUDED.repository_id, \
           routing_mode = EXCLUDED.routing_mode, \
           prompt = EXCLUDED.prompt, \
           tag_ids = EXCLUDED.tag_ids, \
           run_as_user_id = EXCLUDED.run_as_user_id, \
           agent_type = EXCLUDED.agent_type, \
           inactive_ttl_days = EXCLUDED.inactive_ttl_days",
    )
    .bind(&trigger_id)
    .bind(&repo.id)
    .bind(resolved.routing_mode)
    .bind(&resolved.prompt)
    .bind(&resolved.tag_ids)
    .bind(&resolved.run_as_user_id)
    .bind(&resolved.agent_type)
    .bind(resolved.inactive_ttl_days)
    .execute(&mut *tx)
    .await?;

    // Replace source rows so a kind switch (github_pr <-> schedule) is clean.
    sqlx::query("DELETE FROM adapter_event_trigger_sources WHERE trigger_id = $1")
        .bind(&trigger_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM schedule_trigger_sources WHERE trigger_id = $1")
        .bind(&trigger_id)
        .execute(&mut *tx)
        .await?;

    if let Some(adapter) = &resolved.adapter {
        sqlx::query(
            "INSERT INTO adapter_event_trigger_sources \
             (trigger_id, adapter_kind, connection_id, source_matcher, source_name, \
              event_filter, adapter_config, allow_direct_send, close_task_on_close, notify_on_success) \
             VALUES ($1, $2, $3, $4, NULL, $5, $6, false, true, false)",
        )
        .bind(&trigger_id)
        .bind(&adapter.adapter_kind)
        .bind(&adapter.connection_id)
        .bind(&adapter.source_matcher)
        .bind(&adapter.event_filter)
        .bind(&adapter.adapter_config)
        .execute(&mut *tx)
        .await?;
    }
    if let Some(schedule) = &resolved.schedule {
        sqlx::query(
            "INSERT INTO schedule_trigger_sources \
             (trigger_id, cron_expression, timezone, message_template, next_fire_at) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(&trigger_id)
        .bind(&schedule.cron_expression)
        .bind(&schedule.timezone)
        .bind(&schedule.message_template)
        .bind(schedule.next_fire_at)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(created)
}

/// Resolve a GitHub token for loop fetching, best-effort.
///
/// Goes through the canonical resolver so App / installation / `secret_ref`
/// rows all yield a usable token. Returns `None` (unauthenticated fetch, works
/// for public repos) when there's no connection or resolution fails.
async fn resolve_github_token(
    state: &AppState,
    repository: &Repository,
    remote_url: &str,
) -> Option<String> {
    match resolve_repository_github_token(state, repository, remote_url).await {
        Ok(token) => token,
        Err(e) => {
            warn!(error = %e, "Loop sync: failed to resolve GitHub token");
            None
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

/// Resolve a loop's `run_as_user_id` in priority order:
/// frontmatter → global config default → repo owner → repo creator.
pub(crate) fn resolve_run_as_user_id(
    frontmatter: Option<&str>,
    config_default: Option<&str>,
    repo_owner: Option<&str>,
    repo_created_by: Option<&str>,
) -> Option<String> {
    frontmatter
        .or(config_default)
        .or(repo_owner)
        .or(repo_created_by)
        .map(str::to_string)
}

/// Resolve a `run_as_user_id` reference — an internal uuid, or a legacy provider
/// id preserved in `user_provider_ids.provider_user_id` — to the internal uuid.
/// Returns `None` when it names no known user.
async fn resolve_run_as_to_internal_id(
    users: &dyn crate::auth::users::UserRepository,
    raw: &str,
) -> Option<String> {
    if let Ok(uid) = raw.parse::<uuid::Uuid>() {
        if matches!(users.get_user(&uid).await, Ok(Some(_))) {
            return Some(uid.to_string());
        }
    }
    match users.find_user_by_legacy_id(raw).await {
        Ok(Some(user)) => Some(user.id.to_string()),
        _ => None,
    }
}

/// Given the repo's existing `repo_loop` triggers as `(trigger_id, loop_name)`
/// and the set of loop names currently present in the tree, return the trigger
/// ids whose loop directory is gone and should be deleted.
pub(crate) fn names_to_delete(
    existing: &[(String, String)],
    present: &HashSet<String>,
) -> Vec<String> {
    existing
        .iter()
        .filter(|(_, loop_name)| !present.contains(loop_name))
        .map(|(id, _)| id.clone())
        .collect()
}

/// Default kickoff message for a schedule loop with no explicit `schedule.message`.
pub(crate) fn default_schedule_message(loop_name: &str) -> String {
    format!("Run the `{loop_name}` loop.")
}

/// Build the `provider_config` (stored as `adapter_config`) for an `on: linear`
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
    fn run_as_prefers_frontmatter() {
        let got =
            resolve_run_as_user_id(Some("u_fm"), Some("u_cfg"), Some("u_owner"), Some("u_cb"));
        assert_eq!(got.as_deref(), Some("u_fm"));
    }

    #[test]
    fn run_as_falls_back_to_config_default() {
        let got = resolve_run_as_user_id(None, Some("u_cfg"), Some("u_owner"), Some("u_cb"));
        assert_eq!(got.as_deref(), Some("u_cfg"));
    }

    #[test]
    fn run_as_falls_back_to_repo_owner_then_creator() {
        assert_eq!(
            resolve_run_as_user_id(None, None, Some("u_owner"), Some("u_cb")).as_deref(),
            Some("u_owner")
        );
        assert_eq!(
            resolve_run_as_user_id(None, None, None, Some("u_cb")).as_deref(),
            Some("u_cb")
        );
        assert_eq!(resolve_run_as_user_id(None, None, None, None), None);
    }

    #[test]
    fn names_to_delete_picks_only_absent_loops() {
        let existing = vec![
            ("tr_1".to_string(), "java-review".to_string()),
            ("tr_2".to_string(), "daily-health-check".to_string()),
            ("tr_3".to_string(), "stale-loop".to_string()),
        ];
        let mut present = HashSet::new();
        present.insert("java-review".to_string());
        present.insert("daily-health-check".to_string());

        let to_delete = names_to_delete(&existing, &present);
        assert_eq!(to_delete, vec!["tr_3".to_string()]);
    }

    #[test]
    fn names_to_delete_empty_present_deletes_all() {
        let existing = vec![("tr_1".to_string(), "a".to_string())];
        let to_delete = names_to_delete(&existing, &HashSet::new());
        assert_eq!(to_delete, vec!["tr_1".to_string()]);
    }

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
            name: None,
            agent_type: None,
            routing_mode: None,
            tags: None,
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
