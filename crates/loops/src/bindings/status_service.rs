//! Live external status for task bindings.
//!
//! Everything adapter-specific — which subject a binding points at, how to
//! reach it, how to normalize what comes back — lives behind the status
//! capability trait. What is left here is what is genuinely shared: a
//! process-wide cache keyed by external subject, the write-through that keeps
//! the persisted value on the binding warm, and the decision of whether a
//! binding is worth polling at all.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use chrono::{DateTime, Utc};
use moka::future::Cache;

use crate::bindings::operations::Bindings;
use moatless_adapter_model::capabilities::AdapterBindingStatus;
use moatless_adapter_model::registry::Adapters;
use moatless_types::adapter::ExternalStatus;
use moatless_types::adapter::TaskBinding;

/// Whether a binding's live external status is worth fetching again: pure,
/// no I/O.
///
/// A blob predating a field the adapter now needs is refreshed once even
/// when terminal; otherwise a terminal binding never re-polls, so a missed
/// webhook leaves it stale forever (no backstop poll).
pub fn should_refresh(
    status: &dyn AdapterBindingStatus,
    binding: &TaskBinding,
    cutoff: DateTime<Utc>,
) -> bool {
    if status.binding_status_key(binding).is_none() {
        return false;
    }
    // A persisted blob missing a field the adapter now writes is refreshed once,
    // even when terminal, so the fetch fills it in.
    if !status.persisted_status_is_complete(binding) {
        return true;
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
/// [`AdapterBindingStatus::binding_status_key`]), so bindings sharing a subject
/// share one entry and API call.
///
/// A short TTL keeps the live fetch cheap under fan-out without holding stale
/// status for long.
fn cache() -> &'static Cache<String, ExternalStatus> {
    static CACHE: OnceLock<Cache<String, ExternalStatus>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Cache::builder()
            .time_to_live(Duration::from_mins(2))
            .max_capacity(10_000)
            .build()
    })
}

/// Process-global cache of subjects whose live fetch just failed (no token,
/// revoked install, deleted subject), keyed the same way as [`cache`].
///
/// A binding that can never resolve has no status [`should_refresh`] can see
/// as stale, so a caller that bypasses it (e.g. `bound_pull_requests`) would
/// re-dial every read without this. Same TTL as the positive cache.
fn failure_cache() -> &'static Cache<String, ()> {
    static CACHE: OnceLock<Cache<String, ()>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Cache::builder()
            .time_to_live(Duration::from_mins(2))
            .max_capacity(10_000)
            .build()
    })
}

/// Fetch the live external status for a binding, caching it and writing it
/// through to the binding's persisted columns.
///
/// `None` when unobtainable (no live-status adapter, config, token, or an
/// API error) — callers fall back to what's persisted. `created_by` resolves
/// a per-user token where the adapter needs one.
pub async fn get_or_fetch(
    adapters: Option<&Arc<Adapters>>,
    task_bindings: &Bindings,
    binding: &TaskBinding,
    created_by: &str,
) -> Option<ExternalStatus> {
    get_or_fetch_over(
        adapters,
        task_bindings,
        binding,
        created_by,
        cache(),
        failure_cache(),
    )
    .await
}

/// [`get_or_fetch`] over explicit caches, so a test can swap in short-TTL
/// instances instead of waiting out the production TTL.
async fn get_or_fetch_over(
    adapters: Option<&Arc<Adapters>>,
    task_bindings: &Bindings,
    binding: &TaskBinding,
    created_by: &str,
    cache: &Cache<String, ExternalStatus>,
    failures: &Cache<String, ()>,
) -> Option<ExternalStatus> {
    let status = adapters?.binding_status_by_name(binding.adapter_kind.as_str())?;
    let key = status.binding_status_key(binding)?;
    if let Some(cached) = cache.get(&key).await {
        return Some(cached);
    }
    if failures.get(&key).await.is_some() {
        return None;
    }

    let Some(fetched) = status.fetch_binding_status(binding, created_by).await else {
        failures.insert(key, ()).await;
        return None;
    };

    cache.insert(key, fetched.clone()).await;

    // Write-through. `TASK_BINDING_SELECT` does not list the `external_status*`
    // columns and `TaskBindingRow` defaults them, so no production read sees
    // what this writes — the `integration_adapters` db-tests are its readers.
    if let Err(e) = task_bindings
        .record_external_status(
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

    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::Duration as ChronoDuration;
    use serde_json::Value;

    use moatless_adapter_model::capabilities::{Adapter, AdapterBindingStatus};
    use moatless_adapter_model::registry::Adapters;
    use moatless_types::adapter::AdapterKind;

    use super::*;

    /// Stand-in adapter answering only the three questions `should_refresh`
    /// asks.
    ///
    /// `keyed` mirrors a real adapter failing to name a binding's subject (no
    /// live status, or a binding it cannot resolve).
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

        /// Mirrors a real adapter whose blob grew a field: a persisted blob
        /// that does not carry `headRef` predates the growth and reads
        /// incomplete.
        fn persisted_status_is_complete(&self, binding: &TaskBinding) -> bool {
            binding
                .external_status_raw
                .as_ref()
                .is_none_or(|raw| raw.get("headRef").and_then(Value::as_str).is_some())
        }
    }

    fn binding(state_kind: Option<&str>, updated_at: Option<DateTime<Utc>>) -> TaskBinding {
        let now = Utc::now();
        TaskBinding {
            id: "b1".into(),
            task_id: "t1".into(),
            adapter_kind: moatless_types::adapter::AdapterKind::Webhook,
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
            watching: true,
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

    #[test]
    fn terminal_binding_with_a_complete_blob_stays_unpolled() {
        let mut binding = binding(Some("done"), Some(Utc::now() - ChronoDuration::days(30)));
        binding.external_status_raw = Some(serde_json::json!({ "headRef": "feature" }));
        assert!(!should_refresh(
            &StubAdapter { keyed: true },
            &binding,
            cutoff()
        ));
    }

    #[test]
    fn terminal_binding_with_an_incomplete_blob_refreshes_once() {
        // The blob predates the field the adapter now writes, so it is refreshed
        // even though its state kind is terminal.
        let mut binding = binding(Some("done"), Some(Utc::now() - ChronoDuration::days(30)));
        binding.external_status_raw = Some(serde_json::json!({ "state": "closed" }));
        assert!(should_refresh(
            &StubAdapter { keyed: true },
            &binding,
            cutoff()
        ));
    }

    /// Stand-in adapter for [`get_or_fetch_over`]: counts every live fetch and
    /// either answers it or reports it unobtainable, mirroring a binding with
    /// no token / a revoked install / a deleted subject.
    struct CountingAdapter {
        calls: Arc<AtomicUsize>,
        fail: bool,
    }

    impl Adapter for CountingAdapter {
        fn adapter_kind(&self) -> AdapterKind {
            AdapterKind::Webhook
        }
    }

    #[async_trait::async_trait]
    impl AdapterBindingStatus for CountingAdapter {
        fn binding_status_key(&self, binding: &TaskBinding) -> Option<String> {
            Some(format!("counting:{}", binding.subject_key))
        }

        async fn fetch_binding_status(
            &self,
            _binding: &TaskBinding,
            _acting_user: &str,
        ) -> Option<ExternalStatus> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                None
            } else {
                Some(ExternalStatus {
                    status: Some("open".into()),
                    state_kind: Some("open".into()),
                    raw: serde_json::json!({}),
                })
            }
        }
    }

    /// An `Arc<Bindings>`-free `Bindings` over a lazily-connecting pool that
    /// never dials out. `get_or_fetch`'s only write, the write-through on a
    /// successful fetch, already tolerates failing (logs, returns the fetch
    /// regardless), so a pool refusing on first use covers every branch
    /// these tests care about without a live Postgres.
    fn unconnected_task_bindings() -> Bindings {
        use sqlx::postgres::PgPoolOptions;
        use std::net::TcpListener;

        // Bind a loopback listener and drop it immediately: the port is then
        // guaranteed nothing is listening on, so a connect attempt is refused
        // right away. A fixed low port (e.g. `127.0.0.1:1`) risks a firewalled
        // or filtered range that silently drops instead of refusing, which
        // turns this into a multi-second hang rather than an instant failure.
        let port = TcpListener::bind("127.0.0.1:0")
            .expect("can bind an ephemeral loopback port")
            .local_addr()
            .expect("a bound listener has a local address")
            .port();
        let pool = PgPoolOptions::new()
            .acquire_timeout(Duration::from_millis(100))
            .connect_lazy(&format!(
                "postgres://postgres:postgres@127.0.0.1:{port}/postgres"
            ))
            .expect("connect_lazy only parses the URL; it never dials out");
        Bindings::new(Arc::new(crate::bindings::dao::PgTaskBindingDao::new(pool)))
    }

    fn adapters_with(stub: Arc<CountingAdapter>) -> Arc<Adapters> {
        let mut adapters = Adapters::new();
        adapters.register_trait_object(Arc::clone(&stub) as Arc<dyn Adapter>);
        adapters
            .register_binding_status(AdapterKind::Webhook, stub as Arc<dyn AdapterBindingStatus>);
        Arc::new(adapters)
    }

    fn caches_with_ttl(ttl: Duration) -> (Cache<String, ExternalStatus>, Cache<String, ()>) {
        (
            Cache::builder().time_to_live(ttl).build(),
            Cache::builder().time_to_live(ttl).build(),
        )
    }

    #[tokio::test]
    async fn a_failed_fetch_is_recorded_and_not_retried_within_the_ttl() {
        let calls = Arc::new(AtomicUsize::new(0));
        let adapters = adapters_with(Arc::new(CountingAdapter {
            calls: calls.clone(),
            fail: true,
        }));
        let task_bindings = unconnected_task_bindings();
        let binding = binding(Some("open"), None);
        let (cache, failures) = caches_with_ttl(Duration::from_millis(80));

        let first = get_or_fetch_over(
            Some(&adapters),
            &task_bindings,
            &binding,
            "user-1",
            &cache,
            &failures,
        )
        .await;
        assert!(first.is_none(), "the stub reports the subject unobtainable");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Within the TTL, a second read must not hit the live fetch again.
        let second = get_or_fetch_over(
            Some(&adapters),
            &task_bindings,
            &binding,
            "user-1",
            &cache,
            &failures,
        )
        .await;
        assert!(second.is_none());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the negative cache should have skipped the live fetch"
        );

        // Past the TTL, the live fetch is tried again.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let third = get_or_fetch_over(
            Some(&adapters),
            &task_bindings,
            &binding,
            "user-1",
            &cache,
            &failures,
        )
        .await;
        assert!(third.is_none());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "the negative cache entry should have expired"
        );
    }

    #[tokio::test]
    async fn a_successful_fetch_is_still_cached_and_not_retried_within_the_ttl() {
        let calls = Arc::new(AtomicUsize::new(0));
        let adapters = adapters_with(Arc::new(CountingAdapter {
            calls: calls.clone(),
            fail: false,
        }));
        let task_bindings = unconnected_task_bindings();
        let binding = binding(Some("open"), None);
        // A long TTL here: this test asserts the positive cache is consulted
        // at all, not that it expires — the write-through's own (fast but
        // nonzero) failed-connection attempt must not race a short one.
        let (cache, failures) = caches_with_ttl(Duration::from_secs(30));

        let first = get_or_fetch_over(
            Some(&adapters),
            &task_bindings,
            &binding,
            "user-1",
            &cache,
            &failures,
        )
        .await;
        assert!(first.is_some());
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let second = get_or_fetch_over(
            Some(&adapters),
            &task_bindings,
            &binding,
            "user-1",
            &cache,
            &failures,
        )
        .await;
        assert!(second.is_some());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the positive cache should have skipped the live fetch"
        );
    }
}
