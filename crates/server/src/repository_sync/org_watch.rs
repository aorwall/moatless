//! The GitHub organization sync, which registers each organization's
//! repositories and so gives each its default Workspace.
//!
//! An organization syncs as soon as its webhook reports a repository created,
//! renamed, transferred or deleted, and otherwise on a fallback interval: long
//! for one whose webhook is live, short for one with no webhook to report.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use moatless_adapter_model::connections::{ConnectionEvent, ConnectionRouting};
use moatless_core::leader::LeaderGuard;
use moatless_types::adapter::AdapterKind;
use moatless_workspaces::repository::org_sync::OrgSync;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// How often the recorded webhook events are read: two database reads, and no
/// GitHub call unless an organization is due.
pub const ORG_WATCH_TICK: Duration = Duration::from_mins(1);

/// How often an organization with no live webhook is listed: one GitHub call
/// per hundred repositories.
pub const ORG_SYNC_INTERVAL: Duration = Duration::from_mins(10);

/// How often an organization whose webhook is live is still listed, for a
/// repository event the hook does not send or a delivery that failed.
pub const COVERED_ORG_SYNC_INTERVAL: Duration = Duration::from_hours(1);

/// When each organization was last listed.
#[derive(Clone, Copy)]
struct Synced {
    at: Instant,
    /// The newest repository event the sync accounted for, on the database's
    /// clock, which stamps every event.
    events_through: DateTime<Utc>,
}

/// Spawn the sync loop, which syncs every organization at once and then as
/// [`due`] decides. A `None` leader syncs on every replica, for a
/// single-replica Docker deployment.
pub fn spawn(
    sync: OrgSync,
    connections: Arc<ConnectionRouting>,
    leader: Option<Arc<dyn LeaderGuard>>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!(orgs = ?sync.orgs(), "GitHub organization sync started");
        // In memory, so a restart or a leadership change lists every
        // organization once.
        let mut synced = HashMap::new();
        loop {
            let leading = match &leader {
                None => true,
                Some(guard) => guard.try_acquire_or_renew().await.unwrap_or_else(|e| {
                    warn!(error = %e, "GitHub organization sync leader election failed");
                    false
                }),
            };
            if leading {
                tick(&sync, &connections, &mut synced, &cancel).await;
            } else {
                synced.clear();
            }
            tokio::select! {
                () = cancel.cancelled() => return,
                () = tokio::time::sleep(ORG_WATCH_TICK) => {}
            }
        }
    })
}

/// Sync every organization that is due, stopping between organizations once
/// `cancel` fires.
async fn tick(
    sync: &OrgSync,
    connections: &ConnectionRouting,
    synced: &mut HashMap<String, Synced>,
    cancel: &CancellationToken,
) {
    let oldest = synced.values().map(|s| s.events_through).min();
    let changes = match oldest {
        Some(since) => {
            read_events(
                connections,
                moatless_adapter_github_pr::REPOSITORY_CHANGE,
                since,
            )
            .await
        }
        None => Vec::new(),
    };
    let window = chrono::Duration::from_std(super::watch::PUSH_COVERAGE_WINDOW)
        .unwrap_or(chrono::Duration::MAX);
    let live: HashSet<String> = read_events(
        connections,
        moatless_adapter_github_pr::DEFAULT_BRANCH_PUSH,
        Utc::now() - window,
    )
    .await
    .iter()
    .map(|event| event.scope_value.to_lowercase())
    .collect();

    for org in sync.orgs() {
        if cancel.is_cancelled() {
            return;
        }
        let key = org.to_lowercase();
        let previous = synced.get(&key).copied();
        let changed = previous.and_then(|s| newest_change(&changes, &key, s.events_through));
        if !due(previous, live.contains(&key), changed, Instant::now()) {
            continue;
        }
        let events_through = changed
            .or(previous.map(|s| s.events_through))
            .unwrap_or_else(Utc::now);
        synced.insert(
            key,
            Synced {
                at: Instant::now(),
                events_through,
            },
        );
        sync_org(sync, org).await;
    }
}

async fn read_events(
    connections: &ConnectionRouting,
    event_type: &str,
    since: DateTime<Utc>,
) -> Vec<ConnectionEvent> {
    connections
        .events_since(AdapterKind::Github.as_str(), event_type, since)
        .await
        .unwrap_or_else(|e| {
            // Reads as no event and no live webhook, so every organization
            // falls back to `ORG_SYNC_INTERVAL`.
            warn!(error = %e, event_type, "Could not read GitHub webhook events");
            Vec::new()
        })
}

/// The newest repository event for organization `key` after `through`.
fn newest_change(
    changes: &[ConnectionEvent],
    key: &str,
    through: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    changes
        .iter()
        .filter(|event| event.scope_value.to_lowercase() == key && event.received_at > through)
        .map(|event| event.received_at)
        .max()
}

/// Whether an organization is listed this tick: never listed since start, a
/// repository event since its last listing, or its fallback interval elapsed.
fn due(previous: Option<Synced>, live: bool, changed: Option<DateTime<Utc>>, now: Instant) -> bool {
    let Some(previous) = previous else {
        return true;
    };
    let interval = if live {
        COVERED_ORG_SYNC_INTERVAL
    } else {
        ORG_SYNC_INTERVAL
    };
    changed.is_some() || now.duration_since(previous.at) >= interval
}

async fn sync_org(sync: &OrgSync, org: &str) {
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

#[cfg(test)]
mod tests {
    use super::{COVERED_ORG_SYNC_INTERVAL, ORG_SYNC_INTERVAL, Synced, due};
    use chrono::Utc;
    use std::time::{Duration, Instant};

    fn synced_ago(now: Instant, ago: Duration) -> Option<Synced> {
        let at = now.checked_sub(ago)?;
        Some(Synced {
            at,
            events_through: Utc::now(),
        })
    }

    #[test]
    fn an_organization_is_listed_at_start_and_on_a_repository_event() {
        let now = Instant::now();
        assert!(due(None, true, None, now));
        assert!(due(
            synced_ago(now, Duration::ZERO),
            true,
            Some(Utc::now()),
            now
        ));
        assert!(!due(synced_ago(now, Duration::ZERO), true, None, now));
    }

    #[test]
    fn a_live_webhook_stretches_the_fallback_interval() {
        let now = Instant::now();
        let after_short = synced_ago(now, ORG_SYNC_INTERVAL);
        assert!(due(after_short, false, None, now));
        assert!(!due(after_short, true, None, now));
        assert!(due(
            synced_ago(now, COVERED_ORG_SYNC_INTERVAL),
            true,
            None,
            now
        ));
    }
}
