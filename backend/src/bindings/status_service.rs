//! Live external status for task bindings.
//!
//! Everything adapter-specific — which subject a binding points at, how to
//! reach it, how to normalize what comes back — lives behind the `Adapter`
//! trait. What is left here is what is genuinely shared: a process-wide cache
//! keyed by external subject, and the write-through that keeps the persisted
//! value on the binding warm.

use std::sync::OnceLock;
use std::time::Duration;

use moka::future::Cache;

use crate::adapters::models::{ExternalStatus, TaskBinding};
use crate::bindings::dao::PgTaskBindingRepository;
use crate::state::AppState;

/// Process-global cache keyed by the external subject a binding points at (see
/// [`crate::adapters::Adapter::binding_status_key`]), so several tasks bound to
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
    let adapter = state.adapters.as_ref()?.get_by_name(&binding.provider)?;
    let key = adapter.binding_status_key(binding)?;
    if let Some(cached) = cache().get(&key).await {
        return Some(cached);
    }

    let fetched = adapter.fetch_binding_status(binding, created_by).await?;

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
