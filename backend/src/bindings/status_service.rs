//! Live external status for task bindings.
//!
//! Everything adapter-specific — which subject a binding points at, how to
//! reach it, how to normalize what comes back — lives behind the status capability
//! trait. What is left here is what is genuinely shared: a process-wide cache
//! keyed by external subject, the write-through that keeps the persisted value
//! on the binding warm, and the decision of whether a binding is worth polling
//! at all.

use std::sync::OnceLock;
use std::time::Duration;

use chrono::{DateTime, Utc};
use moka::future::Cache;

use crate::adapters::models::{ExternalStatus, TaskBinding};
use crate::adapters::AdapterBindingStatus;
use crate::bindings::dao::PgTaskBindingDao;
use crate::state::AppState;

/// Whether a binding's live external status is worth fetching again.
///
/// Pure: an adapter, a binding and a staleness cutoff in, a decision out — no
/// database, no HTTP, no application state. A binding is refreshed when all
/// three hold:
///
/// - the adapter can name the external subject the binding points at (adapters
///   without live status, and bindings the adapter cannot resolve, say `None`);
/// - the persisted state kind is not terminal — a merged pull request or a
///   completed issue can no longer change on its own, so its persisted status
///   stands and is never polled again;
/// - the persisted status is older than `cutoff`, or was never fetched.
///
/// The terminal rule is a *polling* rule only. Webhook persistence stays
/// unconditional, so a reopened pull request or a reactivated issue overwrites
/// a terminal status and becomes refreshable again. If that webhook is missed
/// nothing re-polls (the GitHub PR status/reaction design — a backstop poll was considered and
/// rejected).
pub(crate) fn should_refresh(
    status: &dyn AdapterBindingStatus,
    binding: &TaskBinding,
    cutoff: DateTime<Utc>,
) -> bool {
    if status.binding_status_key(binding).is_none() {
        return false;
    }
    let terminal = binding
        .external_state_kind
        .as_deref()
        .is_some_and(|kind| status.is_terminal_state_kind(kind));
    if terminal {
        return false;
    }
    binding
        .external_status_updated_at
        .is_none_or(|updated_at| updated_at < cutoff)
}

/// Process-global cache keyed by the external subject a binding points at (see
/// [`crate::adapters::AdapterBindingStatus::binding_status_key`]), so several tasks bound to
/// the same subject share one entry and one API call. A short TTL keeps the
/// live fetch cheap under fan-out without holding stale status for long.
fn cache() -> &'static Cache<String, ExternalStatus> {
    static CACHE: OnceLock<Cache<String, ExternalStatus>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Cache::builder()
            .time_to_live(Duration::from_mins(2))
            .max_capacity(10_000)
            .build()
    })
}

/// Fetch the live external status for a binding, caching the result and writing
/// it through to the binding's persisted columns. Returns `None` when no live
/// status can be obtained (adapter without live status, no adapter configured,
/// no token, API error) — callers fall back to whatever is already persisted.
///
/// `created_by` is the owning task's creator, used by adapters that resolve a
/// per-user token.
pub async fn get_or_fetch(
    state: &AppState,
    binding: &TaskBinding,
    created_by: &str,
) -> Option<ExternalStatus> {
    let status = state
        .adapters
        .as_ref()?
        .binding_status_by_name(&binding.adapter_kind)?;
    let key = status.binding_status_key(binding)?;
    if let Some(cached) = cache().get(&key).await {
        return Some(cached);
    }

    let fetched = status.fetch_binding_status(binding, created_by).await?;

    cache().insert(key, fetched.clone()).await;

    // Write-through so the persisted value stays warm for DB-only reads.
    if let Err(e) = PgTaskBindingDao(state.db.clone())
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

#[cfg(test)]
mod tests {

    use chrono::Duration as ChronoDuration;

    use crate::adapters::AdapterBindingStatus;

    use super::*;

    /// Stand-in adapter: answers the three interface questions the refresh
    /// decision asks, and nothing else. `keyed` mirrors a real adapter failing
    /// to name a binding's subject (no live status, or a binding it cannot
    /// resolve).
    struct StubAdapter {
        keyed: bool,
    }

    #[async_trait::async_trait]
    impl AdapterBindingStatus for StubAdapter {
        fn binding_status_key(&self, binding: &TaskBinding) -> Option<String> {
            self.keyed.then(|| format!("stub:{}", binding.subject_key))
        }

        fn is_terminal_state_kind(&self, state_kind: &str) -> bool {
            state_kind == "done"
        }
    }

    fn binding(state_kind: Option<&str>, updated_at: Option<DateTime<Utc>>) -> TaskBinding {
        let now = Utc::now();
        TaskBinding {
            id: "b1".into(),
            task_id: "t1".into(),
            adapter_kind: "stub".into(),
            subject_kind: "stub_subject".into(),
            subject_key: "acme/web#7".into(),
            connection_id: "c1".into(),
            loop_id: None,
            destination_config: serde_json::json!({}),
            metadata: serde_json::json!({}),
            last_external_event_id: None,
            origin: "ingress".into(),
            created_at: now,
            last_event_at: now,
            external_status: state_kind.map(str::to_string),
            external_state_kind: state_kind.map(str::to_string),
            external_status_raw: None,
            external_review_decision: None,
            external_mergeable_state: None,
            external_status_updated_at: updated_at,
            active: true,
            deactivated_at: None,
        }
    }

    /// The staleness cutoff the read path uses: statuses older than this are
    /// stale.
    fn cutoff() -> DateTime<Utc> {
        Utc::now() - ChronoDuration::seconds(300)
    }

    #[test]
    fn terminal_state_kind_is_never_polled() {
        // Stale by a mile, but finished — its persisted status stands.
        let binding = binding(Some("done"), Some(Utc::now() - ChronoDuration::days(30)));
        assert!(!should_refresh(
            &StubAdapter { keyed: true },
            &binding,
            cutoff()
        ));
    }

    #[test]
    fn binding_the_adapter_cannot_name_is_skipped() {
        let binding = binding(Some("open"), None);
        assert!(!should_refresh(
            &StubAdapter { keyed: false },
            &binding,
            cutoff()
        ));
    }

    #[test]
    fn fresh_non_terminal_binding_is_skipped() {
        let binding = binding(Some("open"), Some(Utc::now() - ChronoDuration::seconds(10)));
        assert!(!should_refresh(
            &StubAdapter { keyed: true },
            &binding,
            cutoff()
        ));
    }

    #[test]
    fn stale_non_terminal_binding_is_refreshed() {
        let binding = binding(Some("open"), Some(Utc::now() - ChronoDuration::hours(1)));
        assert!(should_refresh(
            &StubAdapter { keyed: true },
            &binding,
            cutoff()
        ));
    }

    #[test]
    fn never_fetched_binding_is_stale() {
        let binding = binding(None, None);
        assert!(should_refresh(
            &StubAdapter { keyed: true },
            &binding,
            cutoff()
        ));
    }

    #[test]
    fn reopened_binding_becomes_refreshable_again() {
        // A webhook overwrote the terminal status unconditionally; polling
        // resumes from the state kind it wrote.
        let mut binding = binding(Some("done"), Some(Utc::now() - ChronoDuration::hours(1)));
        assert!(!should_refresh(
            &StubAdapter { keyed: true },
            &binding,
            cutoff()
        ));

        binding.external_state_kind = Some("open".into());
        assert!(should_refresh(
            &StubAdapter { keyed: true },
            &binding,
            cutoff()
        ));
    }
}
