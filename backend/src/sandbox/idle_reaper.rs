//! Idle sandbox reaper.
//!
//! Background job that periodically checks all running sandboxes and stops
//! any that have had no new messages within a configurable timeout. Uses
//! K8s Lease-based leader election so only one replica runs the reaper.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use k8s_openapi::api::apps::v1::Deployment;
use kube::api::{Api, ListParams};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::k8s::client::K8sClient;
use super::k8s::leader_election::LeaderElection;
use super::provider::SandboxProvider;
use crate::config::Config;
use crate::message::dao::MessageRepository;
use crate::task::dao::TaskRepository;

/// Errors that can occur during a reap cycle.
#[derive(Debug, thiserror::Error)]
enum ReaperError {
    #[error("Kubernetes API error: {0}")]
    Kube(#[from] kube::Error),
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}

/// Spawn the idle sandbox reaper as a background tokio task.
///
/// The reaper runs in a loop:
/// 1. Try to acquire leadership via K8s Lease (`"moatless-idle-reaper"`).
/// 2. If leader, run a reap cycle.
/// 3. Sleep for `config.sandbox.idle_check_interval_secs` before the next cycle.
pub fn spawn_idle_reaper(
    k8s: K8sClient,
    messages: Arc<dyn MessageRepository>,
    tasks: Arc<dyn TaskRepository>,
    sandbox: Arc<dyn SandboxProvider>,
    config: Arc<Config>,
    cancel: CancellationToken,
) -> JoinHandle<()> {
    let identity = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());
    let interval = Duration::from_secs(config.sandbox.idle_check_interval_secs);
    let default_ttl_minutes = config.sandbox.idle_timeout_minutes as i32;

    tokio::spawn(async move {
        // Lease duration must exceed the check interval so the lease
        // doesn't expire between cycles. Use 2x the interval as buffer.
        let lease_duration_secs = (config.sandbox.idle_check_interval_secs * 2) as i32;
        let leader_election = LeaderElection::new_with_name(
            &k8s,
            identity.clone(),
            "moatless-idle-reaper".to_string(),
        )
        .with_lease_duration(lease_duration_secs);

        loop {
            if cancel.is_cancelled() {
                info!("Idle reaper shutting down (cancellation)");
                return;
            }

            // Try to acquire or renew leadership
            match leader_election.try_acquire_or_renew().await {
                Ok(true) => {
                    debug!(identity = %identity, "Idle reaper is leader, running reap cycle");
                    if let Err(e) = reap_idle_sandboxes(
                        &k8s,
                        &messages,
                        &tasks,
                        &sandbox,
                        default_ttl_minutes,
                    )
                    .await
                    {
                        warn!(error = %e, "Idle reaper cycle failed");
                    }
                }
                Ok(false) => {
                    debug!(identity = %identity, "Idle reaper not leader, skipping cycle");
                }
                Err(e) => {
                    warn!(error = %e, "Idle reaper leader election error");
                }
            }

            // Wait before next cycle
            tokio::select! {
                () = cancel.cancelled() => {
                    info!("Idle reaper shutting down (cancellation)");
                    return;
                }
                () = tokio::time::sleep(interval) => {}
            }
        }
    })
}

/// Run one reap cycle: list running sandboxes, check idle times, stop idle ones.
async fn reap_idle_sandboxes(
    k8s: &K8sClient,
    messages: &Arc<dyn MessageRepository>,
    tasks: &Arc<dyn TaskRepository>,
    sandbox: &Arc<dyn SandboxProvider>,
    default_ttl_minutes: i32,
) -> Result<(), ReaperError> {
    let task_ids = list_running_sandbox_task_ids(k8s).await?;

    if task_ids.is_empty() {
        debug!("No running sandboxes found");
        return Ok(());
    }

    debug!(count = task_ids.len(), "Found running sandboxes");

    // Batch-query latest message timestamps
    let mut latest_times = messages.get_latest_message_times(&task_ids).await?;

    // Overlay sandbox_started_at — prevents killing just-restarted sandboxes
    match tasks.get_sandbox_started_times(&task_ids).await {
        Ok(started_times) => {
            for (task_id, started_at) in started_times {
                latest_times
                    .entry(task_id)
                    .and_modify(|existing| {
                        if started_at > *existing {
                            *existing = started_at;
                        }
                    })
                    .or_insert(started_at);
            }
        }
        Err(e) => {
            warn!(error = %e, "Failed to fetch sandbox start times, using message times only");
        }
    }

    // Fill creation times for tasks with no messages (prevents killing new tasks)
    let no_message_ids: Vec<String> = task_ids
        .iter()
        .filter(|id| !latest_times.contains_key(*id))
        .cloned()
        .collect();

    if !no_message_ids.is_empty() {
        match tasks.get_created_times(&no_message_ids).await {
            Ok(created_times) => {
                for (task_id, created_at) in created_times {
                    latest_times.entry(task_id).or_insert(created_at);
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to fetch task creation times, skipping no-message tasks");
            }
        }
    }

    // Per-task TTL: rows missing from the map fall back to `default_ttl_minutes`.
    let ttl_map = tasks
        .get_ttl_minutes_map(&task_ids)
        .await
        .unwrap_or_else(|e| {
            warn!(error = %e, "Failed to fetch ttl_minutes map, falling back to default for all tasks");
            HashMap::new()
        });

    let now = Utc::now();
    let mut stopped_count = 0u32;

    for task_id in &task_ids {
        let ttl_minutes = ttl_map
            .get(task_id)
            .copied()
            .unwrap_or(default_ttl_minutes);
        let latest_at = latest_times.get(task_id).copied();

        match decide_reap(ttl_minutes, latest_at, now) {
            ReapDecision::Skip(reason) => {
                debug!(task_id = %task_id, reason, "Skipping reap");
            }
            ReapDecision::Keep => {}
            ReapDecision::Stop => {
                info!(
                    task_id = %task_id,
                    ttl_minutes,
                    last_activity = ?latest_at,
                    "Stopping idle sandbox"
                );
                match sandbox.stop(task_id).await {
                    Ok(()) => {
                        info!(task_id = %task_id, "Idle sandbox stopped successfully");
                        stopped_count += 1;
                    }
                    Err(e) => {
                        warn!(task_id = %task_id, error = %e, "Failed to stop idle sandbox");
                    }
                }
            }
        }
    }

    if stopped_count > 0 {
        info!(stopped_count, "Idle reaper cycle complete");
    } else {
        debug!("Idle reaper cycle complete, no sandboxes stopped");
    }

    Ok(())
}

/// Outcome of evaluating one task in a reap cycle.
#[derive(Debug, PartialEq, Eq)]
enum ReapDecision {
    /// Sandbox is idle past its TTL — stop it.
    Stop,
    /// Sandbox is still within its TTL — leave it running.
    Keep,
    /// Cannot decide (no activity timestamp, or TTL disabled). Reason is for logs.
    Skip(&'static str),
}

/// Pure decision function: decide whether to reap a single task.
///
/// `ttl_minutes == 0` means the task is pinned indefinitely — never reap.
/// A missing `latest_at` means we have no activity floor (no messages, no
/// sandbox-started timestamp, no creation time) and skip rather than risk
/// killing a brand-new sandbox.
fn decide_reap(
    ttl_minutes: i32,
    latest_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> ReapDecision {
    if ttl_minutes <= 0 {
        return ReapDecision::Skip("ttl_minutes is 0 (indefinite)");
    }
    let Some(latest_at) = latest_at else {
        return ReapDecision::Skip("no activity timestamp");
    };
    let cutoff = now - chrono::Duration::minutes(ttl_minutes as i64);
    if latest_at < cutoff {
        ReapDecision::Stop
    } else {
        ReapDecision::Keep
    }
}

/// List task IDs of all running sandbox Deployments (replicas > 0).
async fn list_running_sandbox_task_ids(k8s: &K8sClient) -> Result<Vec<String>, kube::Error> {
    let deployments: Api<Deployment> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let lp = ListParams::default().labels("app.kubernetes.io/managed-by=moatless");

    let list = deployments.list(&lp).await?;

    let task_ids: Vec<String> = list
        .items
        .into_iter()
        .filter(|dep| dep.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1) > 0)
        .filter_map(|dep| {
            dep.metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get("moatless.io/task-id").cloned())
        })
        .collect();

    Ok(task_ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(seconds_ago: i64) -> DateTime<Utc> {
        Utc::now() - chrono::Duration::seconds(seconds_ago)
    }

    #[test]
    fn ttl_zero_is_indefinite() {
        let now = Utc::now();
        let very_old = now - chrono::Duration::days(7);
        let decision = decide_reap(0, Some(very_old), now);
        assert_eq!(decision, ReapDecision::Skip("ttl_minutes is 0 (indefinite)"));
    }

    #[test]
    fn negative_ttl_is_treated_as_indefinite() {
        let now = Utc::now();
        let very_old = now - chrono::Duration::days(7);
        let decision = decide_reap(-1, Some(very_old), now);
        assert_eq!(decision, ReapDecision::Skip("ttl_minutes is 0 (indefinite)"));
    }

    #[test]
    fn missing_activity_skips() {
        let now = Utc::now();
        let decision = decide_reap(60, None, now);
        assert_eq!(decision, ReapDecision::Skip("no activity timestamp"));
    }

    #[test]
    fn within_ttl_keeps() {
        let now = Utc::now();
        // Last activity 30 min ago, ttl=60 → keep.
        let decision = decide_reap(60, Some(ts(30 * 60)), now);
        assert_eq!(decision, ReapDecision::Keep);
    }

    #[test]
    fn beyond_ttl_stops() {
        let now = Utc::now();
        // Last activity 90 min ago, ttl=60 → stop.
        let decision = decide_reap(60, Some(ts(90 * 60)), now);
        assert_eq!(decision, ReapDecision::Stop);
    }

    #[test]
    fn pinned_12h_keeps_after_2h_idle() {
        let now = Utc::now();
        // 12h pin, last activity 2h ago → keep.
        let decision = decide_reap(720, Some(ts(2 * 60 * 60)), now);
        assert_eq!(decision, ReapDecision::Keep);
    }

    #[test]
    fn pinned_12h_stops_after_13h_idle() {
        let now = Utc::now();
        let decision = decide_reap(720, Some(ts(13 * 60 * 60)), now);
        assert_eq!(decision, ReapDecision::Stop);
    }
}
