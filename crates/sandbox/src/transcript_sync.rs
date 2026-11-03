//! Agent session-state sync: backend-proxied S3 persistence of CLI resume data.
//!
//! Claude, Codex, Pi and mini-swe sessions survive pod recreation through the backend
//! archive. Only session directories are archived; agent configuration and
//! credentials stay out. I/O is proxied through JWT-authed gRPC.

use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::agent::harness::trajectory::ClaudeSessionFormat;
use crate::config::Config;
use crate::state::SandboxState;

/// Bound on the sync-on-shutdown flush, one of the steps `crate::shutdown`
/// fits inside the pod's termination grace period.
pub(crate) const SHUTDOWN_FLUSH_TIMEOUT: Duration = Duration::from_secs(12);
const CLAUDE_ARCHIVE_PREFIX: &str = "claude";
const CODEX_SESSIONS_ARCHIVE_PREFIX: &str = "codex/sessions";
const MINI_SWE_SESSIONS_ARCHIVE_PREFIX: &str = "mini-swe/sessions";
const PI_SESSIONS_ARCHIVE_PREFIX: &str = "pi/sessions";

/// Resolve the transcript dir: `<claude_config_dir>/projects/<dashed-cwd>`.
///
/// Mirrors the Claude CLI's own `projects/<dashed cwd>` scheme, so sync
/// and `--resume` target the same files; canonicalizes with a lexical fallback
/// matching the SDK's `resolve_project_dir` (identical for a non-symlink path).
pub fn transcript_dir(config: &Config) -> PathBuf {
    Path::new(&config.claude_config_dir)
        .join("projects")
        .join(ClaudeSessionFormat::project_dir(Path::new(
            &config.workspace_dir,
        )))
}

/// Resolve the Codex resumable session dir: `<codex_home_dir>/sessions`.
pub fn codex_sessions_dir(config: &Config) -> PathBuf {
    Path::new(&config.codex_home_dir).join("sessions")
}

/// Resolve the Pi resumable session dir: `<pi_home_dir>/sessions`.
pub(crate) fn pi_sessions_dir(config: &Config) -> PathBuf {
    Path::new(&config.pi_home_dir).join("sessions")
}

/// Resolve the mini-swe resumable journal directory.
pub(crate) fn mini_swe_sessions_dir(config: &Config) -> PathBuf {
    config.mini_swe_home_dir().join("sessions")
}

/// Download the stored session-state archive and restore it.
///
/// Runs after the repo clone, before Ready, so `recover_interrupted_run` and
/// pending-message dispatch see restored `--resume` history. Non-fatal: skips
/// on a missing archive or disabled feature.
pub async fn download_on_start(state: &SandboxState) {
    let task_id = &state.config.task_id;
    let claude_dir = transcript_dir(&state.config);
    let codex_dir = codex_sessions_dir(&state.config);
    let pi_dir = pi_sessions_dir(&state.config);
    let mini_swe_dir = mini_swe_sessions_dir(&state.config);

    for dir in [&claude_dir, &codex_dir, &pi_dir, &mini_swe_dir] {
        if let Err(e) = tokio::fs::create_dir_all(dir).await {
            warn!(task_id = %task_id, dir = %dir.display(), error = %e, "Failed to create session-state dir");
            return;
        }
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

    match extract_tar(
        tar,
        claude_dir.clone(),
        codex_dir.clone(),
        pi_dir.clone(),
        mini_swe_dir.clone(),
    )
    .await
    {
        Ok(()) => {
            info!(
                task_id = %task_id,
                claude_dir = %claude_dir.display(),
                codex_dir = %codex_dir.display(),
                pi_dir = %pi_dir.display(),
                "Restored session state from backend"
            );
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to extract session state (non-fatal)");
        }
    }
}

/// Tar the CLI session-state dirs and upload them to the backend. No-op when
/// all dirs are absent or empty. Best-effort: failures (including a disabled
/// feature) are logged, never propagated.
pub async fn upload(state: &SandboxState) {
    let task_id = &state.config.task_id;
    let claude_dir = transcript_dir(&state.config);
    let codex_dir = codex_sessions_dir(&state.config);
    let pi_dir = pi_sessions_dir(&state.config);
    let mini_swe_dir = mini_swe_sessions_dir(&state.config);

    let Some(tar) = build_tar(
        claude_dir.clone(),
        codex_dir.clone(),
        pi_dir.clone(),
        mini_swe_dir.clone(),
    )
    .await
    else {
        debug!(
            task_id = %task_id,
            claude_dir = %claude_dir.display(),
            codex_dir = %codex_dir.display(),
            pi_dir = %pi_dir.display(),
            "No session state to upload"
        );
        return;
    };

    match state.backend_client.upload_transcript(task_id, tar).await {
        Ok(()) => debug!(task_id = %task_id, "Uploaded session state to backend"),
        Err(e) if is_feature_disabled(&e) => {
            debug!(task_id = %task_id, "Session-state sync unavailable (backend S3 not configured); skipping upload");
        }
        Err(e) => warn!(task_id = %task_id, error = %e, "Session-state upload failed (non-fatal)"),
    }
}

/// Flush the transcript on shutdown, bounded by [`SHUTDOWN_FLUSH_TIMEOUT`].
///
/// Called from the shutdown driver after the agent has stopped (so the active
/// `.jsonl` is flushed) and before the lifecycle token is cancelled; a timeout
/// is logged and swallowed so the drain proceeds.
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

/// Periodically upload the session state until cancelled. Mirrors the
/// credential refresher: `tokio::select!` on the lifecycle token and a fixed
/// interval.
pub async fn run_transcript_sync(
    state: SandboxState,
    cancel: CancellationToken,
    interval: Duration,
) {
    info!(
        task_id = %state.config.task_id,
        interval_secs = interval.as_secs(),
        "Session-state sync started"
    );

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!(task_id = %state.config.task_id, "Session-state sync shutting down");
                return;
            }
            () = tokio::time::sleep(interval) => {
                upload(&state).await;
            }
        }
    }
}

/// True when a gRPC error means the transcript feature is simply unavailable
/// (backend S3 not configured) rather than a real transient failure — so
/// callers can log at debug instead of warn.
fn is_feature_disabled(status: &tonic::Status) -> bool {
    matches!(
        status.code(),
        tonic::Code::FailedPrecondition | tonic::Code::Unimplemented
    )
}

/// Build an in-memory tar of the CLI session-state dirs, or `None` when none
/// has entries.
///
/// Entries are stored under stable top-level prefixes so the archive can
/// restore each CLI location-independently on the next pod.
async fn build_tar(
    claude_dir: PathBuf,
    codex_dir: PathBuf,
    pi_dir: PathBuf,
    mini_swe_dir: PathBuf,
) -> Option<Vec<u8>> {
    let has_claude_entries = has_entries(&claude_dir).await;
    let has_codex_entries = has_entries(&codex_dir).await;
    let has_pi_entries = has_entries(&pi_dir).await;
    let has_mini_swe_entries = has_entries(&mini_swe_dir).await;
    if !has_claude_entries && !has_codex_entries && !has_pi_entries && !has_mini_swe_entries {
        return None;
    }

    tokio::task::spawn_blocking(move || {
        let mut builder = tar::Builder::new(Vec::new());
        if has_claude_entries {
            builder
                .append_dir_all(CLAUDE_ARCHIVE_PREFIX, &claude_dir)
                .ok()?;
        }
        if has_codex_entries {
            builder
                .append_dir_all(CODEX_SESSIONS_ARCHIVE_PREFIX, &codex_dir)
                .ok()?;
        }
        if has_pi_entries {
            builder
                .append_dir_all(PI_SESSIONS_ARCHIVE_PREFIX, &pi_dir)
                .ok()?;
        }
        if has_mini_swe_entries {
            builder
                .append_dir_all(MINI_SWE_SESSIONS_ARCHIVE_PREFIX, &mini_swe_dir)
                .ok()?;
        }
        builder.into_inner().ok()
    })
    .await
    .ok()
    .flatten()
}

async fn has_entries(dir: &Path) -> bool {
    match tokio::fs::read_dir(&dir).await {
        Ok(mut rd) => matches!(rd.next_entry().await, Ok(Some(_))),
        Err(_) => false,
    }
}

/// Untar `bytes` into the appropriate CLI dirs on a blocking thread.
/// Prefix-routed archives route by top-level dir; flat archives restore into
/// the Claude dir.
async fn extract_tar(
    bytes: Vec<u8>,
    claude_dir: PathBuf,
    codex_dir: PathBuf,
    pi_dir: PathBuf,
    mini_swe_dir: PathBuf,
) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || {
        if !has_combined_archive_prefixes(&bytes)? {
            let mut archive = tar::Archive::new(std::io::Cursor::new(bytes));
            return archive.unpack(&claude_dir);
        }

        let mut archive = tar::Archive::new(std::io::Cursor::new(bytes));
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.into_owned();
            let target = if let Ok(rel) = path.strip_prefix(CLAUDE_ARCHIVE_PREFIX) {
                safe_join(&claude_dir, rel)
            } else if let Ok(rel) = path.strip_prefix(CODEX_SESSIONS_ARCHIVE_PREFIX) {
                safe_join(&codex_dir, rel)
            } else if let Ok(rel) = path.strip_prefix(PI_SESSIONS_ARCHIVE_PREFIX) {
                safe_join(&pi_dir, rel)
            } else if let Ok(rel) = path.strip_prefix(MINI_SWE_SESSIONS_ARCHIVE_PREFIX) {
                safe_join(&mini_swe_dir, rel)
            } else {
                None
            };
            if let Some(target) = target {
                entry.unpack(target)?;
            }
        }
        Ok(())
    })
    .await
    .map_err(std::io::Error::other)?
}

fn has_combined_archive_prefixes(bytes: &[u8]) -> std::io::Result<bool> {
    let mut archive = tar::Archive::new(std::io::Cursor::new(bytes));
    for entry in archive.entries()? {
        let entry = entry?;
        let path = entry.path()?;
        if path.starts_with(CLAUDE_ARCHIVE_PREFIX)
            || path.starts_with(CODEX_SESSIONS_ARCHIVE_PREFIX)
            || path.starts_with(PI_SESSIONS_ARCHIVE_PREFIX)
            || path.starts_with(MINI_SWE_SESSIONS_ARCHIVE_PREFIX)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn safe_join(base: &Path, rel: &Path) -> Option<PathBuf> {
    if rel.as_os_str().is_empty() {
        return None;
    }
    if rel.components().any(|c| {
        matches!(
            c,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return None;
    }
    Some(base.join(rel))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pi_only_archive_restores_a_session_larger_than_the_runtime_mount() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = crate::config::test_support::minimal_config();
        config.pi_home_dir = tmp.path().join("pi-agent").display().to_string();
        let pi_src = pi_sessions_dir(&config);
        tokio::fs::create_dir_all(&pi_src).await.unwrap();
        let session = format!(
            "{{\"type\":\"message\",\"text\":\"{}\"}}\n",
            "x".repeat(2 * 1024 * 1024)
        );
        tokio::fs::write(pi_src.join("session.jsonl"), &session)
            .await
            .unwrap();
        tokio::fs::write(Path::new(&config.pi_home_dir).join("auth.json"), "private")
            .await
            .unwrap();

        let tar = build_tar(
            tmp.path().join("no-claude"),
            tmp.path().join("no-codex"),
            pi_src,
            tmp.path().join("mini-swe"),
        )
        .await
        .expect("Pi sessions alone must trigger an upload");
        let paths: Vec<_> = tar::Archive::new(std::io::Cursor::new(&tar))
            .entries()
            .unwrap()
            .map(|entry| entry.unwrap().path().unwrap().into_owned())
            .collect();
        assert!(
            paths
                .iter()
                .all(|path| path.starts_with(PI_SESSIONS_ARCHIVE_PREFIX))
        );
        assert!(!paths.iter().any(|path| path.ends_with("auth.json")));

        let pi_dst = tmp.path().join("restored-pi-sessions");
        tokio::fs::create_dir_all(&pi_dst).await.unwrap();
        extract_tar(
            tar,
            tmp.path().join("restored-claude"),
            tmp.path().join("restored-codex"),
            pi_dst.clone(),
            tmp.path().join("mini-swe"),
        )
        .await
        .unwrap();
        assert_eq!(
            tokio::fs::read_to_string(pi_dst.join("session.jsonl"))
                .await
                .unwrap(),
            session
        );
    }

    #[tokio::test]
    async fn mini_swe_only_archive_restores_a_session_larger_than_the_runtime_mount() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut config = crate::config::test_support::minimal_config();
        config.home_dir = tmp.path().join("home").display().to_string();
        let mini_src = mini_swe_sessions_dir(&config);
        tokio::fs::create_dir_all(&mini_src).await.unwrap();
        let session = format!(
            "{{\"type\":\"message\",\"text\":\"{}\"}}\n",
            "x".repeat(2 * 1024 * 1024)
        );
        tokio::fs::write(mini_src.join("session.jsonl"), &session)
            .await
            .unwrap();
        tokio::fs::write(config.mini_swe_home_dir().join("auth.json"), "private")
            .await
            .unwrap();

        let tar = build_tar(
            tmp.path().join("no-claude"),
            tmp.path().join("no-codex"),
            tmp.path().join("no-pi"),
            mini_src,
        )
        .await
        .expect("mini-swe sessions alone must trigger an upload");
        let paths: Vec<_> = tar::Archive::new(std::io::Cursor::new(&tar))
            .entries()
            .unwrap()
            .map(|entry| entry.unwrap().path().unwrap().into_owned())
            .collect();
        assert!(
            paths
                .iter()
                .all(|path| path.starts_with(MINI_SWE_SESSIONS_ARCHIVE_PREFIX))
        );
        assert!(!paths.iter().any(|path| path.ends_with("auth.json")));

        let mini_dst = tmp.path().join("restored-mini-sessions");
        tokio::fs::create_dir_all(&mini_dst).await.unwrap();
        extract_tar(
            tar,
            tmp.path().join("restored-claude"),
            tmp.path().join("restored-codex"),
            tmp.path().join("restored-pi"),
            mini_dst.clone(),
        )
        .await
        .unwrap();
        assert_eq!(
            tokio::fs::read_to_string(mini_dst.join("session.jsonl"))
                .await
                .unwrap(),
            session
        );
    }

    #[tokio::test]
    async fn tar_round_trip_preserves_claude_transcripts_and_codex_sessions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let claude_src = tmp.path().join("projects").join("-workspace");
        let codex_src = tmp.path().join("codex").join("sessions");
        let codex_nested = codex_src.join("2026").join("07").join("15");
        tokio::fs::create_dir_all(&claude_src).await.unwrap();
        tokio::fs::create_dir_all(&codex_nested).await.unwrap();
        tokio::fs::write(claude_src.join("a.jsonl"), b"{\"turn\":1}\n")
            .await
            .unwrap();
        tokio::fs::write(
            codex_nested.join("rollout-abc.jsonl"),
            b"{\"thread\":\"abc\"}\n",
        )
        .await
        .unwrap();

        let tar = build_tar(
            claude_src.clone(),
            codex_src.clone(),
            tmp.path().join("pi"),
            tmp.path().join("mini-swe"),
        )
        .await
        .expect("tar built");
        assert!(!tar.is_empty());

        let claude_dst = tmp.path().join("restored-claude");
        let codex_dst = tmp.path().join("restored-codex-sessions");
        tokio::fs::create_dir_all(&claude_dst).await.unwrap();
        tokio::fs::create_dir_all(&codex_dst).await.unwrap();
        extract_tar(
            tar,
            claude_dst.clone(),
            codex_dst.clone(),
            tmp.path().join("pi"),
            tmp.path().join("mini-swe"),
        )
        .await
        .unwrap();

        assert_eq!(
            tokio::fs::read_to_string(claude_dst.join("a.jsonl"))
                .await
                .unwrap(),
            "{\"turn\":1}\n"
        );
        assert_eq!(
            tokio::fs::read_to_string(
                codex_dst
                    .join("2026")
                    .join("07")
                    .join("15")
                    .join("rollout-abc.jsonl")
            )
            .await
            .unwrap(),
            "{\"thread\":\"abc\"}\n"
        );
    }

    #[tokio::test]
    async fn legacy_flat_archive_restores_into_claude_transcript_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let src = tmp.path().join("legacy");
        tokio::fs::create_dir_all(&src).await.unwrap();
        tokio::fs::write(src.join("legacy.jsonl"), b"{\"turn\":1}\n")
            .await
            .unwrap();

        let tar = tokio::task::spawn_blocking({
            let src = src.clone();
            move || {
                let mut builder = tar::Builder::new(Vec::new());
                builder.append_dir_all(".", &src).unwrap();
                builder.into_inner().unwrap()
            }
        })
        .await
        .unwrap();
        assert!(!tar.is_empty());

        let claude_dst = tmp.path().join("restored-claude");
        let codex_dst = tmp.path().join("restored-codex");
        tokio::fs::create_dir_all(&claude_dst).await.unwrap();
        tokio::fs::create_dir_all(&codex_dst).await.unwrap();
        extract_tar(
            tar,
            claude_dst.clone(),
            codex_dst.clone(),
            tmp.path().join("pi"),
            tmp.path().join("mini-swe"),
        )
        .await
        .unwrap();

        assert_eq!(
            tokio::fs::read_to_string(claude_dst.join("legacy.jsonl"))
                .await
                .unwrap(),
            "{\"turn\":1}\n"
        );
        assert!(
            tokio::fs::read_dir(&codex_dst)
                .await
                .unwrap()
                .next_entry()
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn build_tar_returns_none_for_missing_or_empty_dir() {
        let tmp = tempfile::TempDir::new().unwrap();

        let missing = tmp.path().join("nope");
        let also_missing = tmp.path().join("also-nope");
        assert!(
            build_tar(
                missing,
                also_missing,
                tmp.path().join("pi"),
                tmp.path().join("mini-swe"),
            )
            .await
            .is_none()
        );

        let empty = tmp.path().join("empty");
        let also_empty = tmp.path().join("also-empty");
        tokio::fs::create_dir_all(&empty).await.unwrap();
        tokio::fs::create_dir_all(&also_empty).await.unwrap();
        assert!(
            build_tar(
                empty,
                also_empty,
                tmp.path().join("pi"),
                tmp.path().join("mini-swe"),
            )
            .await
            .is_none()
        );
    }
}
