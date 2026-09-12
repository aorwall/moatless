//! The leader loop and the watch stream it runs under.

use super::event::process_watcher_event;
use super::sweep::{gc_failed_pods, retry_pending_auto_stops, sweep_pending_pods};
use super::*;

/// Dependencies the watcher needs to remediate a crash-looping sandbox. The
/// lifecycle owns plan construction and intent, so the watcher only carries the
/// handle. All are optional because the watcher also runs in setups without a
/// database (where remediation degrades to publishing a `Failed` event / raw
/// scale-to-0).
#[derive(Clone)]
pub(super) struct WatcherDeps {
    /// Enables ConfigMap-backed node archive tracking for node-local workspace
    /// and Docker volumes. When false, the watcher performs no archive writes.
    pub(super) node_archive_enabled: bool,
    /// Shared durable lifecycle handle — the sole writer of Sandbox intent. The
    /// watcher drives crash auto-stop (`stop`) through it. `None` in DB-less
    /// setups, where crash mitigation degrades to a raw scale-to-0.
    pub(super) lifecycle: Option<Arc<Lifecycle>>,
    /// Attach-failure remediation. When a pvc+s3 pod stalls Pending on a volume
    /// attach failure, the watcher asks the runtime to rebuild it on an emptyDir
    /// that restores from S3. `None` disables the fallback (the pod keeps
    /// surfacing the environment error).
    pub(super) storage_fallback: Option<Arc<dyn StorageFallback>>,
}

/// Spawn the pod lifecycle watcher as a background tokio task.
///
/// The watcher runs in a loop:
/// 1. Try to acquire leadership via K8s Lease.
/// 2. If leader, start watching pods.
/// 3. If not leader, sleep and retry.
/// 4. On watch stream error, re-enter the leadership loop.
// reason: the watcher wires seven collaborators (client, publisher, cancel, the
// crash threshold, archive feature gate, the lifecycle handle, and the storage
// fallback) into one spawned task.
#[allow(clippy::too_many_arguments)]
pub fn spawn_pod_lifecycle_watcher(
    k8s: K8sClient,
    event_publisher: Arc<dyn EventPublisher>,
    cancel: CancellationToken,
    max_crash_restart_count: i32,
    node_archive_enabled: bool,
    lifecycle: Option<Arc<Lifecycle>>,
    storage_fallback: Option<Arc<dyn StorageFallback>>,
) -> JoinHandle<()> {
    let identity = std::env::var("HOSTNAME").unwrap_or_else(|_| "unknown".to_string());
    metrics::describe_counter!(
        "sandbox_container_terminations_total",
        "Non-intentional sandbox container terminations, labelled by lifecycle_event, K8s reason, and exit_code"
    );
    metrics::describe_histogram!(
        "moatless_sandbox_startup_seconds",
        "Sandbox startup latency in seconds, measured from pod creation to the sandbox container's first Ready"
    );
    let deps = WatcherDeps {
        node_archive_enabled,
        lifecycle,
        storage_fallback,
    };

    tokio::spawn(async move {
        let leader_election = LeaderElection::new(&k8s, identity.clone());

        loop {
            if cancel.is_cancelled() {
                info!("Pod watcher shutting down (cancellation)");
                return;
            }

            // Try to acquire or renew leadership
            match leader_election.try_acquire_or_renew().await {
                Ok(true) => {
                    info!(identity = %identity, "Leader acquired, starting pod watch");
                    run_pod_watch(
                        &k8s,
                        event_publisher.as_ref(),
                        &leader_election,
                        &cancel,
                        max_crash_restart_count,
                        &deps,
                    )
                    .await;
                    // Watch ended (stream error or leadership lost) — re-enter loop
                    info!("Pod watch ended, re-entering leadership loop");
                }
                Ok(false) => {
                    debug!(identity = %identity, "Not leader, waiting...");
                }
                Err(e) => {
                    warn!(error = %e, "Leader election error");
                }
            }

            // Wait before retrying (also serves as renew interval for non-leaders)
            tokio::select! {
                () = cancel.cancelled() => {
                    info!("Pod watcher shutting down (cancellation)");
                    return;
                }
                () = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
            }
        }
    })
}

/// Run the pod watch stream until it ends or leadership is lost.
async fn run_pod_watch(
    k8s: &K8sClient,
    event_publisher: &dyn EventPublisher,
    leader_election: &LeaderElection,
    cancel: &CancellationToken,
    max_crash_restart_count: i32,
    deps: &WatcherDeps,
) {
    let pods_api: Api<Pod> = Api::namespaced(k8s.client.clone(), k8s.namespace());
    let wc = WatcherConfig::default().labels("app.kubernetes.io/managed-by=moatless");

    let stream = watcher::watcher(pods_api, wc);
    tokio::pin!(stream);

    let mut pod_states: HashMap<String, PodObservedState> = HashMap::new();
    let mut renew_interval = tokio::time::interval(std::time::Duration::from_secs(5));

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                return;
            }
            _ = renew_interval.tick() => {
                // Renew leadership periodically
                match leader_election.try_acquire_or_renew().await {
                    Ok(true) => {} // Still leader
                    Ok(false) => {
                        warn!("Lost leadership, stopping pod watch");
                        return;
                    }
                    Err(e) => {
                        warn!(error = %e, "Leader renewal failed, stopping pod watch");
                        return;
                    }
                }
                // Time-based sweep: the watch stream only fires on pod changes, so
                // a pod that sits Pending or ContainerCreating for minutes produces
                // no events. This tick turns those silent windows into
                // scheduling_blocked / startup_slow events (fire-once per pod).
                sweep_pending_pods(k8s, event_publisher, &mut pod_states, deps).await;
                gc_failed_pods(k8s, &mut pod_states).await;
                retry_pending_auto_stops(&mut pod_states, deps).await;
            }
            event = stream.try_next() => {
                match event {
                    Ok(Some(watcher_event)) => {
                        // Boxed: the handler embeds the (large) auto-disable +
                        // redeploy path; keep this loop's future small.
                        Box::pin(process_watcher_event(
                            k8s,
                            &watcher_event,
                            &mut pod_states,
                            event_publisher,
                            max_crash_restart_count,
                            deps,
                        )).await;
                    }
                    Ok(None) => {
                        info!("Pod watch stream ended");
                        return;
                    }
                    Err(e) => {
                        warn!(error = %e, "Pod watch stream error");
                        return;
                    }
                }
            }
        }
    }
}
