//! Prometheus metrics for sandbox storage backends.
//!
//! Compares storage combinations ([`crate::storage::StorageBackend`]) head-
//! to-head — PVC vs. node-local `emptyDir`, each with/without the S3
//! snapshot.
//!
//! Labels: `outcome` ∈ {`ok`, `error`}; `direction` ∈ {`upload`, `download`,
//! `other`}; `format` ∈ {`tar`, `restic`, `other`}.
//!
//! Provision/restart durations come from the backend; workspace-sync
//! duration/bytes come from the sandbox via `ReportWorkspaceSync` instead.

use std::time::Duration;

use super::storage::StorageBackend;

/// Wall-clock duration of a `provision` call by `storage_backend` and `outcome`
/// (histogram).
///
/// With the PVC switch on this includes creating/attaching it; the emptyDir
/// path skips that entirely.
pub(crate) const SANDBOX_PROVISION_DURATION_SECONDS: &str =
    "moatless_sandbox_provision_duration_seconds";

/// Wall-clock duration of a `restart` call by `storage_backend` (histogram).
/// With the PVC switch on, restart pays a pod-drain wait here to release the
/// `ReadWriteOnce` PVC before the new pod can attach; without it there is none.
pub(crate) const SANDBOX_RESTART_DURATION_SECONDS: &str =
    "moatless_sandbox_restart_duration_seconds";

/// Duration of one workspace sync in seconds by `direction` (histogram).
/// Reported by the sandbox whenever the S3 switch is on.
pub(crate) const WORKSPACE_SYNC_DURATION_SECONDS: &str = "moatless_workspace_sync_duration_seconds";

/// Size of one workspace sync tar in bytes by `direction` (histogram). The
/// `_sum` gives total bytes moved (rate → bytes/s); buckets show the size
/// distribution. Reported by the sandbox whenever the S3 switch is on.
pub(crate) const WORKSPACE_SYNC_BYTES: &str = "moatless_workspace_sync_bytes";

/// Workspace sync outcomes by `direction` and `outcome` (`ok` | `error`)
/// (counter). Reported by the sandbox whenever the S3 switch is on.
pub(crate) const WORKSPACE_SYNC_TOTAL: &str = "moatless_workspace_sync_total";

/// Cold-transition outcomes by `outcome` (counter): `reclaimed` (claim gone),
/// `rescued` (claim gone after upload), `blocked` (claim kept — archive
/// unproven or rescue failed), `skipped` (nothing to do).
pub(crate) const STORAGE_RECLAIM_TOTAL: &str = "moatless_sandbox_storage_reclaim_total";

/// Repairs a sandbox sweep or reconciler performed, by `repair` (counter).
/// A label that stays at zero names a sweep whose race no longer happens.
pub(crate) const SANDBOX_REPAIRS_TOTAL: &str = "moatless_sandbox_repairs_total";

/// An at-least-once repair: a sweep redoing what a primary path lost.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repair {
    /// The reconciler re-sent a pending-message wake to a Ready sandbox.
    Renotify,
    /// An evicted pod left in `Failed` was deleted.
    FailedPodGc,
    /// A crash auto-stop the watch path failed to issue succeeded on a sweep.
    CrashAutoStopRetry,
    /// A pod wedged on a volume attach was rebuilt on `emptyDir` from S3.
    StorageFallback,
    /// A missing Service was recreated for a living Deployment.
    NetworkHeal,
}

impl Repair {
    /// The `repair` label value.
    pub fn as_label(self) -> &'static str {
        match self {
            Self::Renotify => "renotify",
            Self::FailedPodGc => "failed_pod_gc",
            Self::CrashAutoStopRetry => "crash_auto_stop_retry",
            Self::StorageFallback => "storage_fallback",
            Self::NetworkHeal => "network_heal",
        }
    }
}

/// Register sandbox storage metric descriptions with the Prometheus recorder.
/// Call once at startup after the recorder is installed.
pub fn describe_sandbox_metrics() {
    metrics::describe_histogram!(
        SANDBOX_PROVISION_DURATION_SECONDS,
        "Duration of a sandbox provision call in seconds by storage_backend and outcome"
    );
    metrics::describe_histogram!(
        SANDBOX_RESTART_DURATION_SECONDS,
        "Duration of a sandbox restart call in seconds by storage_backend"
    );
    metrics::describe_histogram!(
        WORKSPACE_SYNC_DURATION_SECONDS,
        "Duration of one workspace S3 sync in seconds by direction"
    );
    metrics::describe_histogram!(
        WORKSPACE_SYNC_BYTES,
        "Size of one workspace S3 sync tar in bytes by direction"
    );
    metrics::describe_counter!(
        WORKSPACE_SYNC_TOTAL,
        "Workspace S3 sync outcomes (ok | error) by direction"
    );
    metrics::describe_counter!(
        STORAGE_RECLAIM_TOTAL,
        "Cold-transition outcomes (reclaimed | rescued | blocked | skipped)"
    );
    metrics::describe_counter!(
        SANDBOX_REPAIRS_TOTAL,
        "Repairs performed by sandbox sweeps and the reconciler, by repair"
    );
}

/// Record one repair a sweep or the reconciler performed.
pub fn record_repair(repair: Repair) {
    metrics::counter!(SANDBOX_REPAIRS_TOTAL, "repair" => repair.as_label()).increment(1);
}

/// Record one cold-transition attempt.
pub fn record_storage_reclaim(outcome: &'static str) {
    metrics::counter!(STORAGE_RECLAIM_TOTAL, "outcome" => outcome).increment(1);
}

/// Record one provision call's duration and outcome.
pub fn record_provision(backend: StorageBackend, outcome: &'static str, elapsed: Duration) {
    metrics::histogram!(
        SANDBOX_PROVISION_DURATION_SECONDS,
        "storage_backend" => backend.as_metric_label(),
        "outcome" => outcome,
    )
    .record(elapsed.as_secs_f64());
}

/// Record one restart call's duration.
pub fn record_restart(backend: StorageBackend, elapsed: Duration) {
    metrics::histogram!(
        SANDBOX_RESTART_DURATION_SECONDS,
        "storage_backend" => backend.as_metric_label(),
    )
    .record(elapsed.as_secs_f64());
}

/// Record one workspace-sync report from the sandbox.
///
/// `direction`/`format` are validated to their known values (else `"other"`) to
/// keep labels bounded; a sandbox old enough to send no format is recorded as
/// `"tar"`, which is what it wrote.
pub fn record_workspace_sync(
    direction: &str,
    format: &str,
    bytes: u64,
    duration: Duration,
    ok: bool,
) {
    let direction: &'static str = match direction {
        "upload" => "upload",
        "download" => "download",
        _ => "other",
    };
    let format: &'static str = match format {
        "" | "tar" => "tar",
        "restic" => "restic",
        _ => "other",
    };
    let outcome = if ok { "ok" } else { "error" };

    metrics::counter!(
        WORKSPACE_SYNC_TOTAL,
        "direction" => direction,
        "format" => format,
        "outcome" => outcome,
    )
    .increment(1);

    // Only record size/latency for successful transfers — a failed transfer's
    // byte count and duration are meaningless (a truncated PUT, a timeout).
    if ok {
        metrics::histogram!(WORKSPACE_SYNC_BYTES, "direction" => direction, "format" => format)
            .record(bytes as f64);
        metrics::histogram!(WORKSPACE_SYNC_DURATION_SECONDS, "direction" => direction, "format" => format)
            .record(duration.as_secs_f64());
    }
}
