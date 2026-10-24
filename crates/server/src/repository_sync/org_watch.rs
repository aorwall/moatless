//! The periodic GitHub organization sync, which registers each organization's
//! repositories and so gives each its default Workspace.

use std::sync::Arc;
use std::time::Duration;

use moatless_core::leader::LeaderGuard;
use moatless_workspaces::repository::org_sync::OrgSync;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// How often each organization is listed: one GitHub call per hundred
/// repositories.
pub const ORG_SYNC_INTERVAL: Duration = Duration::from_mins(10);

/// Spawn the sync loop, which syncs at once and then every `interval`. A
/// `None` leader syncs on every replica, for a single-replica Docker
/// deployment.
pub fn spawn(
    sync: OrgSync,
    leader: Option<Arc<dyn LeaderGuard>>,
    interval: Duration,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!(orgs = ?sync.orgs(), "GitHub organization sync started");
        loop {
            let leading = match &leader {
                None => true,
                Some(guard) => guard.try_acquire_or_renew().await.unwrap_or_else(|e| {
                    warn!(error = %e, "GitHub organization sync leader election failed");
                    false
                }),
            };
            if leading {
                tick(&sync, &cancel).await;
            }
            tokio::select! {
                () = cancel.cancelled() => return,
                () = tokio::time::sleep(interval) => {}
            }
        }
    })
}

/// Sync every organization once, stopping between organizations once `cancel`
/// fires.
async fn tick(sync: &OrgSync, cancel: &CancellationToken) {
    for org in sync.orgs() {
        if cancel.is_cancelled() {
            return;
        }
        match sync.sync_org(org).await {
            Ok(summary) => {
                for (repository, error) in &summary.failed {
                    warn!(
                        org,
                        repository, error, "GitHub organization sync failed a repository"
                    );
                }
                for (repository, why) in &summary.kept {
                    info!(
                        org,
                        repository, why, "Kept a repository the listing left out"
                    );
                }
                info!(
                    org,
                    registered = summary.registered,
                    refreshed = summary.refreshed,
                    removed = summary.removed,
                    kept = summary.kept.len(),
                    failed = summary.failed.len(),
                    "GitHub organization sync complete"
                );
            }
            Err(e) => warn!(org, error = %e, "GitHub organization sync failed; nothing changed"),
        }
    }
}
