//! Claude transcript sync — backend-proxied S3 persistence of `projects/<dashed-cwd>/`.
//!
//! Claude writes conversation transcripts (the only stateful, growing part of
//! `$CLAUDE_CONFIG_DIR`) under `projects/<dashed-cwd>/*.jsonl`. Everything else in
//! the config dir is regenerated on every boot, so this is all we persist.
//!
//! The sandbox is AWS-credential-less by design, so transcript I/O is proxied
//! through the backend over JWT-authed gRPC (mirroring `DownloadUpload`). This
//! module owns:
//! - resolving the on-disk transcript dir (mirrors the Claude CLI / SDK),
//! - tarring it for upload and untarring a downloaded archive,
//! - the download-on-start, periodic-upload, and sync-on-shutdown entry points.
//!
//! Every operation is best-effort and tier-agnostic: a task runs fine without
//! resume history, so failures are logged, never fatal. For the ephemeral (no-EBS)
//! tier this sync is the *only* transcript durability.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::config::Config;
use crate::state::SandboxState;

/// Default periodic-upload interval when `TRANSCRIPT_SYNC_INTERVAL_SECS` is unset.
const DEFAULT_INTERVAL_SECS: u64 = 60;

/// Bound on the sync-on-shutdown flush so it stays within the pod's 30s grace
/// period — a stuck upload must never block the drain.
const SHUTDOWN_FLUSH_TIMEOUT: Duration = Duration::from_secs(12);

/// Read the periodic upload interval from `TRANSCRIPT_SYNC_INTERVAL_SECS`
/// (default 60s). Returns `None` when set to `0`, which disables the periodic
/// loop (download-on-start and sync-on-shutdown still run).
pub fn read_interval_from_env() -> Option<Duration> {
    match std::env::var("TRANSCRIPT_SYNC_INTERVAL_SECS") {
        Ok(v) => match v.parse::<u64>() {
            Ok(0) => None,
            Ok(secs) => Some(Duration::from_secs(secs)),
            Err(_) => Some(Duration::from_secs(DEFAULT_INTERVAL_SECS)),
        },
        Err(_) => Some(Duration::from_secs(DEFAULT_INTERVAL_SECS)),
    }
}

/// Resolve the transcript dir: `<claude_config_dir>/projects/<dashed-cwd>`.
///
/// The Claude CLI writes transcripts under `projects/<cwd with '/' → '-'>`, where
/// `cwd` is the canonicalized workspace dir (the agent's `cwd` is
/// `config.workspace_dir`). We mirror that exactly so the sync targets the same
/// files the CLI writes and `--resume` reads. Canonicalize with a lexical fallback,
/// matching the SDK's `resolve_project_dir`; for a non-symlink path the two yield
/// the identical dashed name.
pub fn transcript_dir(config: &Config) -> PathBuf {
    let cwd = PathBuf::from(&config.workspace_dir);
    let resolved = cwd.canonicalize().map_or_else(
        |_| config.workspace_dir.clone(),
        |p| p.to_string_lossy().into_owned(),
    );
    let dashed = resolved.replace('/', "-");
    Path::new(&config.claude_config_dir)
        .join("projects")
        .join(dashed)
}

/// Download the stored transcript and untar it into the transcript dir.
///
/// Runs on startup, after the repo clone and before the sandbox is marked Ready,
/// so `recover_interrupted_run` / pending-message dispatch see restored
/// `--resume` history. Creates the target dir first (the CLI expects it to exist).
/// Non-fatal: a missing transcript or a disabled feature is logged and skipped.
pub async fn download_on_start(state: &SandboxState) {
    let task_id = &state.config.task_id;
    let dir = transcript_dir(&state.config);

    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        warn!(task_id = %task_id, dir = %dir.display(), error = %e, "Failed to create transcript dir");
        return;
    }

    let tar = match state.backend_client.download_transcript(task_id).await {
        Ok(bytes) => bytes,
        Err(e) if is_feature_disabled(&e) => {
            debug!(task_id = %task_id, "Transcript sync unavailable (backend S3 not configured); skipping restore");
            return;
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Transcript download failed (non-fatal)");
            return;
        }
    };

    if tar.is_empty() {
        debug!(task_id = %task_id, "No stored transcript to restore");
        return;
    }

    match extract_tar(tar, dir.clone()).await {
        Ok(()) => {
            info!(task_id = %task_id, dir = %dir.display(), "Restored transcript from backend");
        }
        Err(e) => {
            warn!(task_id = %task_id, dir = %dir.display(), error = %e, "Failed to extract transcript (non-fatal)");
        }
    }
}

/// Tar the transcript dir and upload it to the backend. No-op when the dir is
/// absent or empty. Best-effort: failures (including a disabled feature) are
/// logged, never propagated.
pub async fn upload(state: &SandboxState) {
    let task_id = &state.config.task_id;
    let dir = transcript_dir(&state.config);

    let Some(tar) = build_tar(dir.clone()).await else {
        debug!(task_id = %task_id, dir = %dir.display(), "No transcripts to upload");
        return;
    };

    match state.backend_client.upload_transcript(task_id, tar).await {
        Ok(()) => debug!(task_id = %task_id, "Uploaded transcript to backend"),
        Err(e) if is_feature_disabled(&e) => {
            debug!(task_id = %task_id, "Transcript sync unavailable (backend S3 not configured); skipping upload");
        }
        Err(e) => warn!(task_id = %task_id, error = %e, "Transcript upload failed (non-fatal)"),
    }
}

/// Flush the transcript on shutdown, bounded by [`SHUTDOWN_FLUSH_TIMEOUT`].
///
/// Called from the shutdown driver after the agent has stopped (so the active
/// `.jsonl` is flushed) and before the lifecycle token is cancelled. A timeout is
/// logged and swallowed so the drain proceeds.
pub async fn flush_on_shutdown(state: &SandboxState) {
    if tokio::time::timeout(SHUTDOWN_FLUSH_TIMEOUT, upload(state))
        .await
        .is_ok()
    {
        info!("Shutdown: flushed transcript to backend");
    } else {
        warn!("Shutdown: transcript flush timed out, continuing");
    }
}

/// Periodically upload the transcript until cancelled. Mirrors the credential
/// refresher: `tokio::select!` on the lifecycle token and a fixed interval.
pub async fn run_transcript_sync(
    state: SandboxState,
    cancel: CancellationToken,
    interval: Duration,
) {
    info!(
        task_id = %state.config.task_id,
        interval_secs = interval.as_secs(),
        "Transcript sync started"
    );

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!(task_id = %state.config.task_id, "Transcript sync shutting down");
                return;
            }
            () = tokio::time::sleep(interval) => {
                upload(&state).await;
            }
        }
    }
}

/// True when a gRPC error means the transcript feature is simply unavailable
/// (backend S3 not configured) rather than a real transient failure — so callers
/// can log at debug instead of warn.
fn is_feature_disabled(status: &tonic::Status) -> bool {
    matches!(
        status.code(),
        tonic::Code::FailedPrecondition | tonic::Code::Unimplemented
    )
}

/// Build an in-memory tar of `dir`, or `None` when it has no entries.
///
/// The tar build uses the blocking `tar` + `std::fs` APIs, so it runs on a
/// blocking thread. Entries are stored relative to `dir` (`./<file>`) so the
/// archive untars location-independently on the next pod.
async fn build_tar(dir: PathBuf) -> Option<Vec<u8>> {
    // Skip when the dir is absent or has no entries — nothing to persist yet.
    let has_entries = match tokio::fs::read_dir(&dir).await {
        Ok(mut rd) => matches!(rd.next_entry().await, Ok(Some(_))),
        Err(_) => false,
    };
    if !has_entries {
        return None;
    }

    tokio::task::spawn_blocking(move || {
        let mut builder = tar::Builder::new(Vec::new());
        builder.append_dir_all(".", &dir).ok()?;
        builder.into_inner().ok()
    })
    .await
    .ok()
    .flatten()
}

/// Untar `bytes` into `dir` on a blocking thread. The `tar` crate rejects
/// path-traversal (`..`) entries by default.
async fn extract_tar(bytes: Vec<u8>, dir: PathBuf) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || {
        let mut archive = tar::Archive::new(std::io::Cursor::new(bytes));
        archive.unpack(&dir)
    })
    .await
    .map_err(std::io::Error::other)?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_interval_defaults_and_disables() {
        // Serialize via a single test since env is process-global.
        std::env::remove_var("TRANSCRIPT_SYNC_INTERVAL_SECS");
        assert_eq!(
            read_interval_from_env(),
            Some(Duration::from_secs(DEFAULT_INTERVAL_SECS))
        );

        std::env::set_var("TRANSCRIPT_SYNC_INTERVAL_SECS", "0");
        assert_eq!(read_interval_from_env(), None);

        std::env::set_var("TRANSCRIPT_SYNC_INTERVAL_SECS", "30");
        assert_eq!(read_interval_from_env(), Some(Duration::from_secs(30)));

        std::env::set_var("TRANSCRIPT_SYNC_INTERVAL_SECS", "garbage");
        assert_eq!(
            read_interval_from_env(),
            Some(Duration::from_secs(DEFAULT_INTERVAL_SECS))
        );

        std::env::remove_var("TRANSCRIPT_SYNC_INTERVAL_SECS");
    }

    #[tokio::test]
    async fn tar_round_trip_preserves_transcripts() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = tmp.path().join("projects").join("-workspace");
        tokio::fs::create_dir_all(&src).await.unwrap();
        tokio::fs::write(src.join("a.jsonl"), b"{\"turn\":1}\n")
            .await
            .unwrap();
        tokio::fs::write(src.join("b.jsonl"), b"{\"turn\":2}\n")
            .await
            .unwrap();

        let tar = build_tar(src.clone()).await.expect("tar built");
        assert!(!tar.is_empty());

        let dst = tmp.path().join("restored");
        tokio::fs::create_dir_all(&dst).await.unwrap();
        extract_tar(tar, dst.clone()).await.unwrap();

        assert_eq!(
            tokio::fs::read_to_string(dst.join("a.jsonl")).await.unwrap(),
            "{\"turn\":1}\n"
        );
        assert_eq!(
            tokio::fs::read_to_string(dst.join("b.jsonl")).await.unwrap(),
            "{\"turn\":2}\n"
        );
    }

    #[tokio::test]
    async fn build_tar_returns_none_for_missing_or_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();

        // Missing dir.
        let missing = tmp.path().join("nope");
        assert!(build_tar(missing).await.is_none());

        // Present but empty dir.
        let empty = tmp.path().join("empty");
        tokio::fs::create_dir_all(&empty).await.unwrap();
        assert!(build_tar(empty).await.is_none());
    }
}
