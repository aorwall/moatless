//! Prometheus metrics for sandbox storage backends.
//!
//! These exist to compare the storage backends (see
//! [`crate::sandbox::k8s::storage::StorageBackend`]) head-to-head — the EBS PVC
//! path vs. node-local paths — so a default change can be made on evidence
//! rather than intuition.
//!
//! ## Cardinality
//! - `storage_backend` ∈ {`ebs`, `ephemeral`, `s3`, `node`} — the handful of backends.
//! - `outcome` ∈ {`ok`, `error`} for provision; the sync counter's `outcome`
//!   is the same two values.
//! - `direction` ∈ {`upload`, `download`}.
//!
//! Provision / restart durations are timed by the backend directly (synchronous
//! K8s calls). Workspace-sync duration/bytes come from the sandbox via
//! `ReportWorkspaceSync` — for the `s3` backend the transfer is sandbox→S3
//! directly, so the backend isn't in the data path and must be told.

use std::time::Duration;

use super::k8s::storage::StorageBackend;

/// Wall-clock duration of a `provision` call by `storage_backend` and `outcome`
/// (histogram). Captures the cost of standing up sandbox resources — for `ebs`
/// this includes creating/attaching the PVC; the node-local backends skip it.
pub(crate) const SANDBOX_PROVISION_DURATION_SECONDS: &str =
    "moatless_sandbox_provision_duration_seconds";

/// Wall-clock duration of a `restart` call by `storage_backend` (histogram).
/// The `ebs` backend pays a pod-drain wait here to release the `ReadWriteOnce`
/// PVC before the new pod can attach; the node-local backends do not.
pub(crate) const SANDBOX_RESTART_DURATION_SECONDS: &str =
    "moatless_sandbox_restart_duration_seconds";

/// Duration of one workspace sync in seconds by `direction` (histogram).
/// Reported by the sandbox for the `s3` backend only.
pub(crate) const WORKSPACE_SYNC_DURATION_SECONDS: &str = "moatless_workspace_sync_duration_seconds";

/// Size of one workspace sync tar in bytes by `direction` (histogram). The
/// `_sum` gives total bytes moved (rate → bytes/s); buckets show the size
/// distribution. Reported by the sandbox for the `s3` backend only.
pub(crate) const WORKSPACE_SYNC_BYTES: &str = "moatless_workspace_sync_bytes";

/// Workspace sync outcomes by `direction` and `outcome` (`ok` | `error`)
/// (counter). Reported by the sandbox for the `s3` backend only.
pub(crate) const WORKSPACE_SYNC_TOTAL: &str = "moatless_workspace_sync_total";

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
}

/// Record one provision call's duration and outcome.
pub(crate) fn record_provision(backend: StorageBackend, outcome: &'static str, elapsed: Duration) {
    metrics::histogram!(
        SANDBOX_PROVISION_DURATION_SECONDS,
        "storage_backend" => backend.as_metric_label(),
        "outcome" => outcome,
    )
    .record(elapsed.as_secs_f64());
}

/// Record one restart call's duration.
pub(crate) fn record_restart(backend: StorageBackend, elapsed: Duration) {
    metrics::histogram!(
        SANDBOX_RESTART_DURATION_SECONDS,
        "storage_backend" => backend.as_metric_label(),
    )
    .record(elapsed.as_secs_f64());
}

/// Record one workspace-sync report from the sandbox.
///
/// `direction` is validated to `"upload"` / `"download"` (anything else →
/// `"other"`) to keep the label bounded.
pub(crate) fn record_workspace_sync(direction: &str, bytes: u64, duration: Duration, ok: bool) {
    let direction: &'static str = match direction {
        "upload" => "upload",
        "download" => "download",
        _ => "other",
    };
    let outcome = if ok { "ok" } else { "error" };

    metrics::counter!(
        WORKSPACE_SYNC_TOTAL,
        "direction" => direction,
        "outcome" => outcome,
    )
    .increment(1);

    // Only record size/latency for successful transfers — a failed transfer's
    // byte count and duration are meaningless (a truncated PUT, a timeout).
    if ok {
        metrics::histogram!(WORKSPACE_SYNC_BYTES, "direction" => direction).record(bytes as f64);
        metrics::histogram!(WORKSPACE_SYNC_DURATION_SECONDS, "direction" => direction)
            .record(duration.as_secs_f64());
    }
}
