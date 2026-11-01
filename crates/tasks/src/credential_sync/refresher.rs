//! Background Codex token refresher.
//!
//! Leader-elected sweep that periodically pre-refreshes Codex tokens before
//! they hit the 8-day refresh-token TTL. Mirrors the shape of the backend's
//! task reaper so a single replica owns the sweep at any time.
//!
//! The actual refresh decision and HTTP call live in
//! [`crate::credential_sync::CredentialSync::ensure_fresh_codex`]; this module
//! just drives the periodic sweep of `provider_tokens`.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::credential_sync::CredentialSync;
use moatless_core::leader::LeaderGuard;
use moatless_ports::identity::CodexScopeReader;
use moatless_types::store::StoreError;

/// Spawn the background Codex refresher as a tokio task.
///
/// Every `interval`, the leader replica sweeps codex scopes with no permanent
/// `refresh_failure` and calls [`CredentialSync::ensure_fresh_codex`], a no-op
/// when the row is still fresh — so the sweeper is idle most of the time.
pub fn spawn_codex_refresher(
    codex_scopes: Arc<dyn CodexScopeReader>,
    credential_sync: Arc<CredentialSync>,
    leader_guard: Arc<dyn LeaderGuard>,
    identity: String,
    interval: Duration,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if cancel.is_cancelled() {
                info!("Codex refresher shutting down (cancellation)");
                return;
            }

            match leader_guard.try_acquire_or_renew().await {
                Ok(true) => {
                    debug!(identity = %identity, "Codex refresher is leader, running sweep");
                    if let Err(e) = run_sweep(codex_scopes.as_ref(), &credential_sync).await {
                        warn!(error = %e, "Codex refresher sweep failed");
                    }
                }
                Ok(false) => {
                    debug!(identity = %identity, "Codex refresher not leader, skipping sweep");
                }
                Err(e) => {
                    warn!(error = %e, "Codex refresher leader election error");
                }
            }

            tokio::select! {
                () = cancel.cancelled() => {
                    info!("Codex refresher shutting down (cancellation)");
                    return;
                }
                () = tokio::time::sleep(interval) => {}
            }
        }
    })
}

async fn run_sweep(
    codex_scopes: &dyn CodexScopeReader,
    credential_sync: &Arc<CredentialSync>,
) -> Result<(), StoreError> {
    // Every healthy codex scope, regardless of which one it is; the refresher
    // then runs the same code path on each.
    let scopes = codex_scopes.healthy_scopes().await?;

    let mut attempted = 0u32;
    let mut errored = 0u32;
    for scope in scopes {
        attempted += 1;
        let scope_label = scope.display();
        if let Err(e) = credential_sync.ensure_fresh_codex(scope).await {
            warn!(error = %e, scope = %scope_label, "Codex refresher: ensure_fresh_codex failed");
            errored += 1;
        }
    }

    if attempted > 0 {
        info!(attempted, errored, "Codex refresher sweep complete");
    }
    Ok(())
}
