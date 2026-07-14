//! External status for task bindings.
//!
//! Two responsibilities:
//!
//! 1. **Pure derivation helpers** ([`github_external_status`],
//!    [`linear_external_status`]) that turn raw PR / issue fields into the
//!    normalized `(display, state_kind, raw)` triple persisted on
//!    `task_bindings`. Shared by the webhook handlers (which already have the
//!    fields) and the live fetch below.
//!
//! 2. A **Moka-cached live fetch** ([`get_or_fetch`]) that, for a binding whose
//!    persisted status is missing/stale, calls the GitHub / Linear API, caches
//!    the result for a short TTL, and writes it through to the binding so the
//!    persisted value stays warm.

use std::sync::OnceLock;
use std::time::Duration;

use moatless_api_schemas::enums::AdapterKind;
use moka::future::Cache;

use crate::adapters::github_pr::adapter::GithubPrAdapter;
use crate::adapters::github_pr::api::{GithubPrApi, PrInfo};
use crate::adapters::linear::adapter::LinearAdapter;
use crate::adapters::models::TaskBinding;
use crate::bindings::dao::PgTaskBindingRepository;
use crate::state::AppState;

/// Normalized external status for a connected subject.
#[derive(Debug, Clone)]
pub struct ExternalStatus {
    /// Display string (GitHub: `open`/`merged`/`closed`/`draft`; Linear: the
    /// workflow state name).
    pub status: Option<String>,
    /// Normalized bucket used for color-coding / filtering.
    pub state_kind: Option<String>,
    /// Full detail blob persisted to `external_status_raw`.
    pub raw: serde_json::Value,
}

/// Derive the normalized status of a GitHub PR from its raw fields.
///
/// GitHub reports both merged and abandoned PRs as `state = "closed"`, so the
/// `merged` flag is needed to distinguish them. Draft is only meaningful while
/// the PR is open.
pub fn github_external_status(
    state: &str,
    merged: bool,
    draft: bool,
    title: Option<&str>,
    url: Option<&str>,
) -> ExternalStatus {
    let kind = if merged {
        "merged"
    } else if state.eq_ignore_ascii_case("closed") {
        "closed"
    } else if draft {
        "draft"
    } else {
        "open"
    };

    ExternalStatus {
        status: Some(kind.to_string()),
        state_kind: Some(kind.to_string()),
        raw: serde_json::json!({
            "title": title,
            "url": url,
            "state": state,
            "merged": merged,
            "draft": draft,
        }),
    }
}

/// Derive the normalized status of a Linear issue from its workflow state.
///
/// The display string is the (user-editable) state name; the normalized bucket
/// is Linear's stable `state.type`.
pub fn linear_external_status(
    state_name: &str,
    state_type: &str,
    title: Option<&str>,
    url: Option<&str>,
) -> ExternalStatus {
    ExternalStatus {
        status: (!state_name.is_empty()).then(|| state_name.to_string()),
        state_kind: (!state_type.is_empty()).then(|| state_type.to_string()),
        raw: serde_json::json!({
            "stateName": state_name,
            "stateType": state_type,
            "title": title,
            "url": url,
        }),
    }
}

/// Derive an [`ExternalStatus`] directly from a fetched [`PrInfo`].
fn status_from_pr_info(pr: &PrInfo) -> ExternalStatus {
    github_external_status(
        &pr.state,
        pr.merged.unwrap_or(false),
        pr.draft.unwrap_or(false),
        Some(pr.title.as_str()),
        pr.html_url.as_deref(),
    )
}

/// Process-global cache keyed by connection identity (`gh:{owner}/{repo}#{n}`,
/// `lin:{issue_id}`). A short TTL keeps the live fetch cheap under fan-out
/// without holding stale status for long.
fn cache() -> &'static Cache<String, ExternalStatus> {
    static CACHE: OnceLock<Cache<String, ExternalStatus>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Cache::builder()
            .time_to_live(Duration::from_mins(2))
            .max_capacity(10_000)
            .build()
    })
}

/// Stable cache key for a binding's connected subject, or `None` when the
/// binding is not one we can resolve a live status for.
fn cache_key(binding: &TaskBinding) -> Option<String> {
    match binding.provider.as_str() {
        "github" | "github_pr" => {
            let (owner, repo, number) = github_subject(binding)?;
            Some(format!("gh:{owner}/{repo}#{number}"))
        }
        "linear" => {
            let issue_id = binding
                .destination_config
                .get("issue_id")
                .and_then(serde_json::Value::as_str)?;
            Some(format!("lin:{issue_id}"))
        }
        _ => None,
    }
}

/// Parse `(owner, repo, pr_number)` from a GitHub binding's destination config.
fn github_subject(binding: &TaskBinding) -> Option<(String, String, u64)> {
    let cfg = &binding.destination_config;
    let owner = cfg.get("owner").and_then(serde_json::Value::as_str)?;
    let repo = cfg.get("repo").and_then(serde_json::Value::as_str)?;
    let number = cfg.get("pr_number").and_then(serde_json::Value::as_u64)?;
    Some((owner.to_string(), repo.to_string(), number))
}

/// Fetch the live external status for a binding, caching the result and writing
/// it through to the binding's persisted columns. Returns `None` when no live
/// status can be obtained (unsupported binding kind, no adapter configured, no
/// token, API error) — callers fall back to whatever is already persisted.
///
/// `created_by` is the owning task's creator, used to resolve a GitHub token
/// scoped to that user.
pub async fn get_or_fetch(
    state: &AppState,
    binding: &TaskBinding,
    created_by: &str,
) -> Option<ExternalStatus> {
    let key = cache_key(binding)?;
    if let Some(cached) = cache().get(&key).await {
        return Some(cached);
    }

    let fetched = match binding.provider.as_str() {
        "github" | "github_pr" => fetch_github(state, binding, created_by).await,
        "linear" => fetch_linear(state, binding).await,
        _ => None,
    }?;

    cache().insert(key, fetched.clone()).await;

    // Write-through so the persisted value stays warm for DB-only reads.
    if let Err(e) = PgTaskBindingRepository(state.db.clone())
        .update_external_status(
            &binding.id,
            fetched.status.as_deref(),
            fetched.state_kind.as_deref(),
            &fetched.raw,
        )
        .await
    {
        tracing::warn!(
            binding_id = %binding.id,
            error = %e,
            "Failed to write-through external status to binding"
        );
    }

    Some(fetched)
}

async fn fetch_github(
    state: &AppState,
    binding: &TaskBinding,
    created_by: &str,
) -> Option<ExternalStatus> {
    let (owner, repo, number) = github_subject(binding)?;
    let adapter = state
        .adapters
        .as_ref()?
        .get(AdapterKind::GithubPr)?
        .as_any()
        .downcast_ref::<GithubPrAdapter>()?;

    let token = match adapter.resolve_token_for_user(created_by, Some(&owner)).await {
        Ok(token) => token,
        Err(e) => {
            tracing::warn!(
                binding_id = %binding.id,
                error = %e,
                "Failed to resolve GitHub token for live PR status"
            );
            return None;
        }
    };

    let api = GithubPrApi::new(&token);
    match api.get_pr(&owner, &repo, number).await {
        Ok(pr) => Some(status_from_pr_info(&pr)),
        Err(e) => {
            tracing::warn!(
                binding_id = %binding.id,
                error = %e,
                "Failed to fetch live GitHub PR status"
            );
            None
        }
    }
}

async fn fetch_linear(state: &AppState, binding: &TaskBinding) -> Option<ExternalStatus> {
    let issue_id = binding
        .destination_config
        .get("issue_id")
        .and_then(serde_json::Value::as_str)?;
    let org_id = binding
        .destination_config
        .get("team_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");

    let adapter = state
        .adapters
        .as_ref()?
        .get(AdapterKind::Linear)?
        .as_any()
        .downcast_ref::<LinearAdapter>()?;

    let api = adapter.agent_api_for_org(org_id);
    match api.get_issue(issue_id).await {
        Ok(issue) => {
            let state_name = issue
                .pointer("/state/name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let state_type = issue
                .pointer("/state/type")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let title = issue.get("title").and_then(serde_json::Value::as_str);
            let url = issue.get("url").and_then(serde_json::Value::as_str);
            Some(linear_external_status(state_name, state_type, title, url))
        }
        Err(e) => {
            tracing::warn!(
                binding_id = %binding.id,
                error = %e,
                "Failed to fetch live Linear issue status"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_merged_beats_closed_and_draft() {
        let s = github_external_status("closed", true, false, Some("t"), Some("u"));
        assert_eq!(s.status.as_deref(), Some("merged"));
        assert_eq!(s.state_kind.as_deref(), Some("merged"));
    }

    #[test]
    fn github_closed_not_merged() {
        let s = github_external_status("closed", false, false, None, None);
        assert_eq!(s.status.as_deref(), Some("closed"));
    }

    #[test]
    fn github_draft_when_open() {
        let s = github_external_status("open", false, true, None, None);
        assert_eq!(s.status.as_deref(), Some("draft"));
        assert_eq!(s.state_kind.as_deref(), Some("draft"));
    }

    #[test]
    fn github_plain_open() {
        let s = github_external_status("open", false, false, None, None);
        assert_eq!(s.status.as_deref(), Some("open"));
    }

    #[test]
    fn linear_uses_name_and_type() {
        let s = linear_external_status("In Review", "started", Some("t"), Some("u"));
        assert_eq!(s.status.as_deref(), Some("In Review"));
        assert_eq!(s.state_kind.as_deref(), Some("started"));
    }

    #[test]
    fn linear_empty_fields_are_none() {
        let s = linear_external_status("", "", None, None);
        assert!(s.status.is_none());
        assert!(s.state_kind.is_none());
    }
}
