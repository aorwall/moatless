//! Background Codex token refresher.
//!
//! Leader-elected sweep that periodically pre-refreshes Codex tokens before
//! they hit the 8-day refresh-token TTL. Mirrors the shape of
//! [`crate::task::reaper::spawn_task_reaper`] so a single replica owns the
//! sweep at any time.
//!
//! The actual refresh decision and HTTP call live in
//! [`crate::credentials::CredentialService::ensure_fresh_codex`]; this module
//! is just the periodic driver that picks rows out of `provider_tokens` and
//! hands each one to the service.

use std::sync::Arc;
use std::time::Duration;

use sqlx::Row;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::credentials::codex::scope::CodexScope;
use crate::credentials::CredentialService;
use crate::db::DbPool;
use crate::sandbox::k8s::client::K8sClient;
use crate::sandbox::k8s::leader_election::LeaderElection;

/// Spawn the background Codex refresher as a tokio task.
///
/// Every `interval`, the leader replica:
/// 1. Queries `provider_tokens` for codex rows that don't have a permanent
///    `refresh_failure` already recorded.
/// 2. For each user, calls
///    [`CredentialService::ensure_fresh_codex`] which is itself a no-op when
///    the row's access token is still fresh.
///
/// The expensive part (`refresh_chatgpt_tokens`) only runs when the row is
/// actually near expiry; the sweeper is effectively idle most of the time.
pub fn spawn_codex_refresher(
    db: DbPool,
    credentials_svc: Arc<CredentialService>,
    k8s: K8sClient,
    interval: Duration,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    let identity = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());

    tokio::spawn(async move {
        // Lease duration must exceed the check interval — use 2x as buffer.
        let lease_duration_secs: i32 = (interval.as_secs() * 2).try_into().unwrap_or(i32::MAX);
        let leader_election = LeaderElection::new_with_name(
            &k8s,
            identity.clone(),
            "moatless-codex-refresher".to_string(),
        )
        .with_lease_duration(lease_duration_secs);

        loop {
            if cancel.is_cancelled() {
                info!("Codex refresher shutting down (cancellation)");
                return;
            }

            match leader_election.try_acquire_or_renew().await {
                Ok(true) => {
                    debug!(identity = %identity, "Codex refresher is leader, running sweep");
                    if let Err(e) = run_sweep(&db, &credentials_svc).await {
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
    db: &DbPool,
    credentials_svc: &Arc<CredentialService>,
) -> Result<(), sqlx::Error> {
    // Reads every healthy codex row regardless of scope. Each row's
    // `(user_id, team_id, scope)` tuple maps to one [`CodexScope`]; the
    // refresher then runs the same code path on each. The partial index
    // `idx_provider_tokens_codex_healthy` (migration `0059`) backs the
    // predicate so this remains O(touched rows).
    let rows = sqlx::query(
        "SELECT user_id, scope FROM provider_tokens \
         WHERE provider = 'codex' \
           AND metadata->>'refresh_failure' IS NULL",
    )
    .fetch_all(db)
    .await?;

    let mut attempted = 0u32;
    let mut errored = 0u32;
    for row in rows {
        let user_id: Option<String> = row.try_get("user_id")?;
        let scope_str: Option<String> = row.try_get("scope")?;

        let scope = match (scope_str.as_deref(), user_id) {
            (Some("user"), Some(uid)) => CodexScope::user(uid),
            (Some("global"), _) => CodexScope::Global,
            other => {
                warn!(
                    ?other,
                    "Codex refresher: skipping row with unrecognized scope shape"
                );
                continue;
            }
        };

        attempted += 1;
        let scope_label = scope.display();
        if let Err(e) = credentials_svc.ensure_fresh_codex(scope, None).await {
            warn!(error = %e, scope = %scope_label, "Codex refresher: ensure_fresh_codex failed");
            errored += 1;
        }
    }

    if attempted > 0 {
        info!(attempted, errored, "Codex refresher sweep complete");
    }
    Ok(())
}
