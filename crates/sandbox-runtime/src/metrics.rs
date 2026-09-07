//! Prometheus metrics for sandbox storage backends.
//!
//! These exist to compare the storage combinations (see
//! [`crate::storage::StorageBackend`]) head-to-head — the PVC path vs. the
//! node-local `emptyDir` path, each with and without the S3 snapshot — so a
//! default change can be made on evidence rather than intuition.
//!
//! ## Cardinality
//! - `storage_backend` ∈ {`pvc+s3`, `pvc`, `s3`, `none`} — one per switch combination.
//! - `outcome` ∈ {`ok`, `error`} for provision; the sync counter's `outcome`
//!   is the same two values.
//! - `direction` ∈ {`upload`, `download`, `other`}.
//! - `format` ∈ {`tar`, `mirror`, `restic`, `other`} — the workspace-snapshot
//!   format the sandbox wrote or read.
//!
//! Provision / restart durations are timed by the backend directly (synchronous
//! K8s calls). Workspace-sync duration/bytes come from the sandbox via
//! `ReportWorkspaceSync` — the transfer is sandbox→S3 directly, so the backend
//! isn't in the data path and must be told.

use std::time::Duration;

use super::storage::StorageBackend;

/// Wall-clock duration of a `provision` call by `storage_backend` and `outcome`
/// (histogram). Captures the cost of standing up sandbox resources — with the
/// PVC switch on this includes creating/attaching it; without it the emptyDir
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

/// Cold-transition outcomes by `outcome` (counter). This is where the cost of
/// the pvc+s3 default is actually paid back, so the four outcomes are kept
/// apart: `reclaimed` (claim gone), `rescued` (claim gone, but only after the
/// volume was uploaded), `blocked` (claim kept — the archive could not be
/// proven current and the rescue did not land), `skipped` (nothing to do).
pub(crate) const STORAGE_RECLAIM_TOTAL: &str = "moatless_sandbox_storage_reclaim_total";

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
/// `direction` is validated to `"upload"` / `"download"` and `format` to
/// `"tar"` / `"mirror"` / `"restic"` (anything else → `"other"`) to keep both
/// labels bounded.
/// A sandbox that predates the file mirror sends no format and is recorded as
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
        "mirror" => "mirror",
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
