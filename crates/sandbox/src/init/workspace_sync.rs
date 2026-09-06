//! Direct-to-S3 persistence of everything under `WORKSPACE_DIR`, enabled by the
//! repo's `s3` storage switch.
//!
//! Inside a git repo the contents come from `git ls-files -c -o
//! --exclude-standard` plus `.git`, so ignored artifacts stay out; outside one
//! everything is included. Entries are stored workspace-relative, so one gzipped
//! tar untars location-independently onto `WORKSPACE_DIR` and reproduces each
//! repo's HEAD, index, branch and dirty tree — which is why [`restore_on_start`]
//! can skip the fresh clone. Restore detects the gzip magic, and handles two
//! older archive shapes: working-tree-only (returns `false`, caller falls back to
//! clone + [`download_on_start`]) and single-repo repo-relative (extracted onto
//! `repo_dir`).
//!
//! The sandbox holds no AWS credentials: the backend mints presigned URLs scoped
//! to this task's `workspace.tar` and stays out of the data path, so the sandbox
//! reports each transfer's size, duration and outcome back for metrics.
//!
//! All four upload points share one [`WorkspaceSyncTracker`], which fingerprints
//! the tracked file set and skips a transfer when nothing moved. The periodic
//! loop is off when the volume is durable ([`is_periodic_enabled`]). Every
//! operation is best-effort: a fresh clone already gives a working repo.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::workspace_mirror;
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::state::SandboxState;

/// The `format` label every transfer this module makes is reported under: it
/// writes and reads one gzipped tar. The file mirror reports `"mirror"`.
const FORMAT: &str = "tar";

/// Default periodic-upload interval when `WORKSPACE_SYNC_INTERVAL_SECS` is unset.
///
/// Five minutes: turn completion and the shutdown flush are the primary
/// durability points, so this loop only bounds the loss from an involuntary
/// mid-turn kill — a coarse cadence is enough and keeps the fingerprint walk +
/// tar upload off the hot path.
const DEFAULT_INTERVAL_SECS: u64 = 300;

/// Bound on the sync-on-shutdown flush so it stays within the pod's 30s grace
/// period (shared with the transcript flush) — a stuck upload must never block
/// the drain.
const SHUTDOWN_FLUSH_TIMEOUT: Duration = Duration::from_secs(10);

/// Leading bytes of a gzip member, used to tell a compressed archive from a
/// plain tar on restore.
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// Per-transfer HTTP timeout for the direct-to-S3 PUT/GET. Generous enough for a
/// large tar on a slow in-cluster link, but bounded so a stuck transfer can't
/// wedge the upload path.
const HTTP_TIMEOUT: Duration = Duration::from_mins(5);

/// What one upload attempt did, for the caller that has to report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyncOutcome {
    /// Whether bytes actually reached S3.
    pub synced: bool,
    /// Size of the uploaded tar; `0` when nothing was uploaded.
    pub bytes: u64,
}

/// The fingerprint of the last successfully uploaded workspace, shared by every
/// upload path.
///
/// Turn boundaries, the pre-stop RPC, the shutdown flush and the periodic loop
/// all ask the same question — *has anything changed since we last shipped?* —
/// so they share one answer. Without that, each path keeps its own baseline and
/// a turn-boundary upload is immediately followed by an identical periodic one.
///
/// The fingerprint is taken *before* the tar is built and stored only after the
/// PUT succeeds, so a change made while the tar was being built is not recorded
/// as shipped: the next attempt uploads again rather than skipping.
#[derive(Clone, Default)]
pub struct WorkspaceSyncTracker {
    last_uploaded: Arc<tokio::sync::Mutex<Option<u64>>>,
    mirror_baseline: Arc<tokio::sync::Mutex<Option<sandbox_proto::workspace_manifest::Manifest>>>,
    /// Written by [`Self::mark_mirror_unusable`], read by [`Self::adopt_current`].
    mirror_unusable: Arc<AtomicBool>,
    turn_upload: Arc<TurnUpload>,
}

/// Admission control for the turn-boundary upload, which runs off the agent's
/// main loop ([`spawn_after_turn`]).
///
/// Turns arrive faster than a large workspace uploads, so spawning one task per
/// turn would pile up transfers of the same tree. At most one upload runs and at
/// most one waits: the upload ships the workspace *as it is when it starts*, not
/// a per-turn delta, so a single trailing run covers every turn that landed
/// while the first was in flight.
#[derive(Default)]
struct TurnUpload {
    /// Held for one upload. Also what makes "one waiting" a real queue rather
    /// than a second concurrent transfer.
    gate: tokio::sync::Mutex<()>,
    /// Set by whoever spawns, cleared by that task once it holds the gate —
    /// never earlier. Clearing it before the gate would let a third turn spawn
    /// behind a queue that is still only one deep, which is the unbounded growth
    /// this exists to prevent.
    queued: AtomicBool,
}

impl WorkspaceSyncTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adopt the current workspace as the baseline without uploading it.
    ///
    /// Called once init has placed the repos: a workspace that is exactly what
    /// the restore or the clone produced has nothing new to ship.
    ///
    /// The two baselines come from different sources because they answer
    /// different questions. The tar gate only decides *whether* to ship, and
    /// what it ships is always a whole archive, so an on-disk walk answers it
    /// regardless of how the tree got there. The mirror baseline is the delta
    /// base, so it has to be what S3 actually holds —
    /// [`workspace_mirror::stored_baseline`].
    pub async fn adopt_current(&self, config: &Config, backend_client: &dyn BackendClient) {
        let dir = PathBuf::from(&config.workspace_dir);
        *self.last_uploaded.lock().await = workspace_fingerprint(dir).await;
        *self.mirror_baseline.lock().await =
            if config.workspace_mirror_enabled && !self.mirror_unusable.load(Ordering::Relaxed) {
                workspace_mirror::stored_baseline(&config.task_id, backend_client).await
            } else {
                None
            };
    }

    /// Record that S3 holds a mirror this sandbox could not restore from.
    ///
    /// Such a manifest names objects S3 may not hold, so it must not become the
    /// delta base: [`Self::adopt_current`] then leaves the base unset and the
    /// next upload ships the whole tree. Without this a task poisoned by a
    /// pre-#686 sandbox never re-uploads what it is missing, because every
    /// unchanged file keeps matching the poisoned manifest.
    pub fn mark_mirror_unusable(&self) {
        self.mirror_unusable.store(true, Ordering::Relaxed);
    }

    /// The baseline the upload gate compares against. `None` means nothing has
    /// been shipped or adopted, so the next attempt uploads.
    #[cfg(test)]
    pub(crate) async fn baseline(&self) -> Option<u64> {
        *self.last_uploaded.lock().await
    }

    /// Whether a mirror delta base is set. Unset means the next upload ships the
    /// whole tree.
    #[cfg(test)]
    pub(crate) async fn has_mirror_baseline(&self) -> bool {
        self.mirror_baseline.lock().await.is_some()
    }

    /// Upload the workspace unless it is unchanged since the last successful
    /// upload. Best-effort: a failed transfer leaves the baseline alone so the
    /// next attempt retries.
    ///
    /// The format is chosen per config: with the mirror on, the per-file mirror
    /// ships the delta, and the tar is written whenever that mirror left no
    /// generation in S3 — an older backend minting no manifest URL, or a failed
    /// transfer — so a mirror outage costs a tar rather than the workspace.
    async fn upload_if_changed(
        &self,
        config: &Config,
        backend_client: &dyn BackendClient,
    ) -> SyncOutcome {
        if config.workspace_mirror_enabled {
            let mut baseline = self.mirror_baseline.lock().await;
            match workspace_mirror::upload(config, backend_client, &mut baseline).await {
                Ok(outcome) => return outcome,
                Err(()) => {
                    debug!(task_id = %config.task_id, "Mirror stored nothing; writing a tar");
                }
            }
        }

        let mut last = self.last_uploaded.lock().await;
        let current = workspace_fingerprint(PathBuf::from(&config.workspace_dir)).await;
        if current.is_none() {
            debug!(task_id = %config.task_id, "Workspace is empty; nothing to upload");
            return SyncOutcome::default();
        }
        if current == *last {
            debug!(task_id = %config.task_id, "Workspace unchanged; skipping upload");
            return SyncOutcome::default();
        }

        let outcome = upload_from_config(config, backend_client).await;
        if outcome.synced {
            *last = current;
        }
        outcome
    }
}

/// Whether the periodic upload loop should run at all.
///
/// It exists to bound the loss when an involuntary kill (node-pressure eviction,
/// OOM, spot reclaim with no grace) destroys the workspace before any other
/// upload path runs. A durable volume survives that kill, so on the PVC tier the
/// accepted bound is the last turn boundary and the loop is pure overhead.
pub fn is_periodic_enabled(config: &Config) -> bool {
    config.workspace_sync_enabled && !config.workspace_restore_if_empty
}

/// Whether the local volume already holds a usable primary repo.
///
/// The test is deliberately the same one the agent would fail on: a `.git` that
/// exists *and* answers `git rev-parse HEAD`. An empty volume, a half-extracted
/// tree, or a `.git` corrupted by a kill mid-write all read as "not usable" and
/// let the snapshot win.
async fn primary_repo_is_usable(repo_dir: &Path) -> bool {
    if !repo_dir.join(".git").exists() {
        return false;
    }
    tokio::process::Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["rev-parse", "HEAD"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|status| status.success())
}

/// What [`restore_on_start`] decided, and what the caller still owes.
///
/// Three states, not two: "the volume already has the repo" and "nothing was
/// restored" both skip the full-snapshot path, but only the latter may be
/// followed by [`download_on_start`]'s working-tree overlay. Collapsing them to
/// one `false` lets an older archive extract over a live durable volume and
/// revert work that was never in the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreDecision {
    /// A full snapshot (carrying `.git`) was extracted. Skip the clone, skip the
    /// overlay — the snapshot *is* the working tree.
    RestoredSnapshot,
    /// The durable volume already holds a usable primary repo. Nothing was
    /// fetched; the volume is the live copy. Skip the overlay — it is the older
    /// copy and would revert the volume.
    UseExistingVolume,
    /// Nothing was restored. Clone as usual, then overlay any working-tree
    /// archive.
    Nothing,
}

impl RestoreDecision {
    /// Whether a full snapshot was extracted — the only case that reports
    /// `RepositoryRestored` and counts as a snapshot origin.
    pub fn restored_snapshot(self) -> bool {
        matches!(self, Self::RestoredSnapshot)
    }

    /// Whether the workspace already holds the authoritative copy, so
    /// [`download_on_start`]'s overlay must not run. True for both the snapshot
    /// we just extracted and the durable volume we chose to keep.
    pub fn skips_overlay(self) -> bool {
        matches!(self, Self::RestoredSnapshot | Self::UseExistingVolume)
    }
}

/// Restore a full snapshot from S3 *before* the clone, returning what the caller
/// still owes (see [`RestoreDecision`]).
///
/// Runs on startup after credentials + git auth are configured but before the
/// repo clone. When a snapshot exists **and contains the primary repo's `.git`**
/// it's a complete set of repos: extracting it onto the empty `emptyDir`
/// reproduces the exact HEAD, index, branch and dirty working tree of every repo
/// from the previous pod (primary *and* siblings), so the caller skips the clone
/// ([`RestoreDecision::RestoredSnapshot`]). This avoids the base-drift
/// phantom-revert bug — there is no fresh clone of an advanced base branch to
/// drift against.
///
/// Returns [`RestoreDecision::UseExistingVolume`] when the volume is durable and
/// already holds a usable primary repo (`WORKSPACE_RESTORE_IF_EMPTY`; nothing is
/// even fetched). The clone is a no-op there, and the overlay is suppressed.
///
/// Returns [`RestoreDecision::Nothing`], leaving the workspace untouched, when:
/// - the feature is disabled (backend S3 not configured),
/// - there is no stored snapshot yet (fresh task → 404),
/// - a transfer/extract fails, or
/// - the snapshot is working-tree-only and carries no `.git`.
///
/// In every `Nothing` case the caller clones as usual, and working-tree-only
/// archives are then overlaid by [`download_on_start`]. Best-effort and
/// non-fatal throughout.
pub async fn restore_on_start(state: &SandboxState) -> RestoreDecision {
    let task_id = &state.config.task_id;
    let workspace_dir = PathBuf::from(&state.config.workspace_dir);
    let repo_dir = PathBuf::from(&state.config.repo_dir);
    let primary_prefix = primary_prefix(&state.config.workspace_dir, &state.config.repo_dir);

    // A durable volume is the live copy: the snapshot is behind it by at most a
    // turn, so extracting it over a populated PVC would silently revert work.
    if state.config.workspace_restore_if_empty && primary_repo_is_usable(&repo_dir).await {
        info!(task_id = %task_id, dir = %repo_dir.display(), "Volume already holds the primary repo; keeping it and skipping the S3 restore");
        return RestoreDecision::UseExistingVolume;
    }

    // The file mirror is the newer format and the one this sandbox writes, so it
    // is tried first; the tar below is what a task last synced by an older
    // sandbox — or by this one with the mirror off — still has.
    if state.config.workspace_mirror_enabled {
        if let Some(decision) = workspace_mirror::restore_on_start(state).await {
            return decision;
        }
    }

    let urls = match state.backend_client.get_workspace_sync_urls(task_id).await {
        Ok(u) => u,
        Err(e) if is_feature_disabled(&e) => {
            debug!(task_id = %task_id, "Workspace sync unavailable (backend S3 not configured); will clone");
            return RestoreDecision::Nothing;
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to mint workspace sync URLs (non-fatal); will clone");
            return RestoreDecision::Nothing;
        }
    };

    let started = Instant::now();
    let client = match http_client(HTTP_TIMEOUT) {
        Ok(client) => client,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Workspace restore client build failed (non-fatal); will clone");
            return RestoreDecision::Nothing;
        }
    };
    let bytes = match http_get(&client, &urls.get_url).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            debug!(task_id = %task_id, "No stored workspace snapshot; will clone");
            return RestoreDecision::Nothing;
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Workspace download failed (non-fatal); will clone");
            state
                .backend_client
                .report_workspace_sync(
                    task_id,
                    "download",
                    FORMAT,
                    0,
                    started.elapsed().as_millis() as u64,
                    Some(&e),
                )
                .await;
            return RestoreDecision::Nothing;
        }
    };

    let size = bytes.len() as u64;
    match extract_workspace(
        bytes,
        workspace_dir.clone(),
        repo_dir.clone(),
        primary_prefix,
    )
    .await
    {
        Ok(true) => {
            info!(task_id = %task_id, bytes = size, dir = %workspace_dir.display(), "Restored repos from S3 (skipping clone)");
            state
                .backend_client
                .report_workspace_sync(
                    task_id,
                    "download",
                    FORMAT,
                    size,
                    started.elapsed().as_millis() as u64,
                    None,
                )
                .await;
            RestoreDecision::RestoredSnapshot
        }
        Ok(false) => {
            debug!(task_id = %task_id, "Snapshot has no primary .git (legacy); will clone then overlay");
            RestoreDecision::Nothing
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to extract repo snapshot (non-fatal); clearing partial state and cloning");
            clear_dir_contents(&repo_dir).await;
            state
                .backend_client
                .report_workspace_sync(
                    task_id,
                    "download",
                    FORMAT,
                    size,
                    started.elapsed().as_millis() as u64,
                    Some(&e.to_string()),
                )
                .await;
            RestoreDecision::Nothing
        }
    }
}

/// Download the stored workspace archive and overlay it onto the fresh clone.
///
/// Only reached when [`restore_on_start`] declined (no snapshot, or a
/// working-tree-only snapshot without `.git`). Runs after the repo clone and
/// before the sandbox is marked Ready, so agent edits / uncommitted changes from
/// the previous pod are in place before the agent runs. The tar is extracted
/// *over* the clone: edited files overwrite their pristine versions, untracked
/// files are added. Working-tree-only archives are repo-relative, so they overlay
/// onto `repo_dir`. Non-fatal: a missing archive or a disabled feature is logged
/// and skipped.
pub async fn download_on_start(state: &SandboxState) {
    let task_id = &state.config.task_id;
    let repo_dir = PathBuf::from(&state.config.repo_dir);

    let urls = match state.backend_client.get_workspace_sync_urls(task_id).await {
        Ok(u) => u,
        Err(e) if is_feature_disabled(&e) => {
            debug!(task_id = %task_id, "Workspace sync unavailable (backend S3 not configured); skipping restore");
            return;
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to mint workspace sync URLs (non-fatal)");
            return;
        }
    };

    let started = Instant::now();
    let client = match http_client(HTTP_TIMEOUT) {
        Ok(client) => client,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Workspace overlay client build failed (non-fatal)");
            return;
        }
    };
    let bytes = match http_get(&client, &urls.get_url).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            debug!(task_id = %task_id, "No stored workspace to restore");
            return;
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Workspace download failed (non-fatal)");
            state
                .backend_client
                .report_workspace_sync(
                    task_id,
                    "download",
                    FORMAT,
                    0,
                    started.elapsed().as_millis() as u64,
                    Some(&e),
                )
                .await;
            return;
        }
    };

    let size = bytes.len() as u64;
    let error = match extract_tar(bytes, repo_dir.clone()).await {
        Ok(()) => {
            info!(task_id = %task_id, bytes = size, dir = %repo_dir.display(), "Restored workspace from S3");
            None
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to extract workspace (non-fatal)");
            Some(e.to_string())
        }
    };
    state
        .backend_client
        .report_workspace_sync(
            task_id,
            "download",
            FORMAT,
            size,
            started.elapsed().as_millis() as u64,
            error.as_deref(),
        )
        .await;
}

/// Tar every git repo under `workspace_dir` and upload it, unless the workspace
/// is unchanged since the last successful upload. No-op when there's nothing to
/// sync. Best-effort: failures (including a disabled feature) are logged, never
/// propagated.
pub async fn upload(state: &SandboxState) -> SyncOutcome {
    state
        .workspace_sync
        .upload_if_changed(&state.config, state.backend_client.as_ref())
        .await
}

/// Start the turn-boundary upload and return; the transfer runs on its own task.
///
/// # Why this does not block its caller
///
/// The caller is the agent coordinator's main loop, the same loop that drains
/// the command channel — user messages, stop requests, and the wake a
/// [`BackgroundWatchJob`](crate::agent::background_watch_job) injects when a
/// `moat cmd` finishes. Awaiting the upload there put every one of those behind
/// an S3 transfer. When the mirror started failing on zero-byte files, each
/// attempt burned its full retry budget and a task sat idle for four minutes
/// with its wake already queued (task `5f0f1eaf`): the agent looked hung, and a
/// user message appeared to be what unstuck it, when it was really the upload
/// finishing.
///
/// Durability is unchanged. The upload still starts at the same boundary, the
/// shared [`WorkspaceSyncTracker`] still gates it on a moved fingerprint, and a
/// failure still leaves the baseline alone. What a slow transfer now delays is
/// only the *next* upload — via [`TurnUpload`] — not the agent.
///
/// [`flush_on_shutdown`] remains the ordered flush: it takes the same tracker
/// locks, so an in-flight background upload either completes first or the flush
/// gives up on its own timeout rather than the two racing.
pub fn spawn_after_turn(
    config: Arc<Config>,
    backend_client: Arc<dyn BackendClient>,
    tracker: WorkspaceSyncTracker,
) {
    // `upload_after_turn` checks this too — it is the gate. Checking here as
    // well just avoids spawning a task per turn to do nothing.
    if !config.workspace_sync_enabled {
        return;
    }
    if tracker.turn_upload.queued.swap(true, Ordering::SeqCst) {
        debug!(task_id = %config.task_id, "Workspace upload already queued; coalescing this turn into it");
        return;
    }
    tokio::spawn(async move {
        let _gate = tracker.turn_upload.gate.lock().await;
        // Whatever landed while we waited is in the tree we are about to read,
        // so it is covered; a turn that lands from here on must queue its own.
        tracker.turn_upload.queued.store(false, Ordering::SeqCst);
        upload_after_turn(&config, backend_client.as_ref(), &tracker).await;
    });
}

/// Upload the workspace when an agent turn reaches a terminal boundary.
///
/// This is the normal durability point between turns — and on the PVC tier the
/// only periodic one, since the loop is off there. Startup restore, the
/// pre-stop RPC and the shutdown flush stay separate.
///
/// The agent reaches this through [`spawn_after_turn`], which runs it off the
/// main loop.
async fn upload_after_turn(
    config: &Config,
    backend_client: &dyn BackendClient,
    tracker: &WorkspaceSyncTracker,
) {
    if config.workspace_sync_enabled {
        tracker.upload_if_changed(config, backend_client).await;
    }
}

/// Answer the backend's pre-stop `SyncWorkspace`: make the durable copy current
/// before the caller destroys the live one.
///
/// Reports `synced: false` both when this sandbox has no S3 switch and when the
/// workspace has not moved since the last upload — in either case S3 already
/// holds what it should and nothing had to travel.
pub async fn sync_on_request(state: &SandboxState) -> SyncOutcome {
    if !state.config.workspace_sync_enabled {
        debug!(task_id = %state.config.task_id, "SyncWorkspace requested but workspace sync is off");
        return SyncOutcome::default();
    }
    upload(state).await
}

/// One-shot rescue upload: tar `workspace_dir` and PUT it to `put_url`.
///
/// The one upload path that reaches neither the backend nor `SandboxState`. Its
/// caller is not a sandbox: it is a short-lived pod the runtime schedules onto a
/// volume whose sandbox is already gone, so there is no task token, no gRPC
/// channel, and nothing to report a metric through — the presigned URL is the
/// whole capability it holds.
///
/// Unlike every other path here it is **not** best-effort. It runs because the
/// backend could not prove the archive current, and its caller deletes the
/// volume on success, so a silent failure would be the one that loses work: an
/// empty workspace, an unreachable URL, and a rejected PUT are all errors.
pub async fn rescue_upload(workspace_dir: &str, put_url: &str) -> Result<u64, String> {
    let tar = build_tar(PathBuf::from(workspace_dir))
        .await
        .ok_or_else(|| format!("no workspace archive could be built from {workspace_dir}"))?;
    let bytes = tar.len() as u64;
    http_put(&http_client(HTTP_TIMEOUT)?, put_url, tar).await?;
    Ok(bytes)
}

async fn upload_from_config(config: &Config, backend_client: &dyn BackendClient) -> SyncOutcome {
    let task_id = &config.task_id;
    let workspace_dir = PathBuf::from(&config.workspace_dir);

    let Some(tar) = build_tar(workspace_dir).await else {
        debug!(task_id = %task_id, "No workspace files to upload");
        return SyncOutcome::default();
    };

    let urls = match backend_client.get_workspace_sync_urls(task_id).await {
        Ok(u) => u,
        Err(e) if is_feature_disabled(&e) => {
            debug!(task_id = %task_id, "Workspace sync unavailable (backend S3 not configured); skipping upload");
            return SyncOutcome::default();
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to mint workspace sync URLs (non-fatal)");
            return SyncOutcome::default();
        }
    };

    let bytes = tar.len() as u64;
    let started = Instant::now();
    let error = match http_client(HTTP_TIMEOUT) {
        Ok(client) => match http_put(&client, &urls.put_url, tar).await {
            Ok(()) => {
                debug!(task_id = %task_id, bytes = bytes, "Uploaded workspace to S3");
                None
            }
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Workspace upload failed (non-fatal)");
                Some(e)
            }
        },
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Workspace upload client build failed (non-fatal)");
            Some(e)
        }
    };
    backend_client
        .report_workspace_sync(
            task_id,
            "upload",
            FORMAT,
            bytes,
            started.elapsed().as_millis() as u64,
            error.as_deref(),
        )
        .await;

    let ok = error.is_none();
    SyncOutcome {
        synced: ok,
        bytes: if ok { bytes } else { 0 },
    }
}

/// Read the periodic upload interval from `WORKSPACE_SYNC_INTERVAL_SECS`
/// (default [`DEFAULT_INTERVAL_SECS`]). Returns `None` when set to `0`, which
/// disables the periodic loop (turn-boundary and shutdown uploads still run).
pub fn read_interval_from_env() -> Option<Duration> {
    match std::env::var("WORKSPACE_SYNC_INTERVAL_SECS") {
        Ok(v) => match v.parse::<u64>() {
            Ok(0) => None,
            Ok(secs) => Some(Duration::from_secs(secs)),
            Err(_) => Some(Duration::from_secs(DEFAULT_INTERVAL_SECS)),
        },
        Err(_) => Some(Duration::from_secs(DEFAULT_INTERVAL_SECS)),
    }
}

/// Periodically upload the workspace, but only when the tracked file set
/// changed since the last upload.
///
/// This is the between-turns durability backstop, and only runs when the volume
/// is not durable (see [`is_periodic_enabled`]): turn completion and the
/// shutdown flush are the primary points, but a long turn killed involuntarily
/// (node-pressure eviction, OOM, spot reclaim with no grace) reaches neither and
/// would otherwise lose everything since the last completed turn.
///
/// Each tick delegates to the shared [`WorkspaceSyncTracker`], which fingerprints
/// the workspace (the same tracked/untracked-non-ignored files plus each repo's
/// `.git` that the tar carries, `.gitignore`d artifacts excluded) and uploads
/// only when it moved — so an idle sandbox, one churning only gitignored build
/// output, or one whose last turn already uploaded does not re-ship a tar.
/// Mirrors the transcript sync: a `tokio::select!` on the lifecycle token and a
/// fixed interval.
pub async fn run_workspace_sync(
    state: SandboxState,
    cancel: CancellationToken,
    interval: Duration,
) {
    let task_id = state.config.task_id.clone();
    info!(
        task_id = %task_id,
        interval_secs = interval.as_secs(),
        "Workspace sync started"
    );

    loop {
        tokio::select! {
            () = cancel.cancelled() => {
                info!(task_id = %task_id, "Workspace sync shutting down");
                return;
            }
            () = tokio::time::sleep(interval) => {
                upload(&state).await;
            }
        }
    }
}

/// Flush the workspace on shutdown, bounded by [`SHUTDOWN_FLUSH_TIMEOUT`].
///
/// Called from the shutdown driver after the agent has stopped (so the working
/// tree is stable) and before the lifecycle token is cancelled. A timeout is
/// logged and swallowed so the drain proceeds.
pub async fn flush_on_shutdown(state: &SandboxState) {
    if tokio::time::timeout(SHUTDOWN_FLUSH_TIMEOUT, upload(state))
        .await
        .is_ok()
    {
        info!("Shutdown: flushed workspace to S3");
    } else {
        warn!("Shutdown: workspace flush timed out, continuing");
    }
}

/// True when a gRPC error means the workspace-sync feature is simply unavailable
/// (backend S3 not configured) rather than a real transient failure.
fn is_feature_disabled(status: &tonic::Status) -> bool {
    matches!(
        status.code(),
        tonic::Code::FailedPrecondition | tonic::Code::Unimplemented
    )
}

/// The workspace-relative prefix of the primary repo (e.g. `"soap-tm"`), or `""`
/// when the primary repo *is* the workspace dir (the no-clone-URL fallback).
pub(super) fn primary_prefix(workspace_dir: &str, repo_dir: &str) -> String {
    match Path::new(repo_dir).strip_prefix(Path::new(workspace_dir)) {
        Ok(rel) => rel.to_string_lossy().trim_end_matches('/').to_string(),
        Err(_) => String::new(),
    }
}

/// Build a reqwest client for transfers bounded by `timeout`.
///
/// The tar paths build one per transfer, which is cheap at the sync cadence
/// (default 5 min) and keeps no long-lived connection open against the
/// presigned host. [`workspace_mirror`](super::workspace_mirror) builds one for
/// a whole sync instead — thousands of objects cannot each pay a handshake.
pub(super) fn http_client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))
}

/// `PUT` bytes to a presigned S3 URL.
///
/// # `Content-Length` is set by hand
///
/// For an empty body reqwest sends neither `Content-Length` nor
/// `Transfer-Encoding`, and S3 answers an unframed `PUT` with `411 Length
/// Required`. The mirror puts one object per file, so a single zero-byte file —
/// a `.gitkeep` is the usual one — failed that sync and every later one, since
/// the manifest is only written once the objects land. Setting the header
/// explicitly frames all three cases: an empty body, a small one, and a large
/// one.
pub(super) async fn http_put(
    client: &reqwest::Client,
    url: &str,
    body: Vec<u8>,
) -> Result<(), String> {
    let resp = client
        .put(url)
        .header(reqwest::header::CONTENT_LENGTH, body.len())
        .body(body)
        .send()
        .await
        .map_err(|e| format!("PUT request failed: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("PUT returned HTTP {}", resp.status()))
    }
}

/// `GET` bytes from a presigned S3 URL. `Ok(None)` on 404 — nothing stored under
/// that key yet.
pub(super) async fn http_get(
    client: &reqwest::Client,
    url: &str,
) -> Result<Option<Vec<u8>>, String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("GET request failed: {e}"))?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(format!("GET returned HTTP {}", resp.status()));
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("reading GET body failed: {e}"))?;
    Ok(Some(bytes.to_vec()))
}

/// The path of `path` relative to `root`, as a `/`-joined string (empty when
/// they are equal). Used to key archive entries at their workspace-relative
/// path.
pub(super) fn rel_from(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .ok()
        .map(|r| r.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Recursively append everything under `dir` to the archive, keyed at paths
/// relative to `root` (the workspace dir). Returns whether anything was added.
///
/// A directory that is a git repo root (contains `.git`) is handed to
/// [`append_repo`], which enumerates it via git so its `.gitignore`d artifacts
/// are excluded while its `.git` is kept — and the generic walk does **not**
/// descend into it (git already listed its contents). Every other file is added
/// verbatim and every other directory is walked, so all non-repo data under the
/// workspace is captured. Symlinks and other non-regular files are skipped
/// (never followed), so the walk can't loop or escape `root`.
fn append_tree<W: std::io::Write>(builder: &mut tar::Builder<W>, root: &Path, dir: &Path) -> bool {
    // A git repo root: enumerate via git (honors gitignore) and stop descending.
    if dir.join(".git").is_dir() {
        return append_repo(builder, dir, &rel_from(root, dir));
    }

    let Ok(entries) = std::fs::read_dir(dir) else {
        return false;
    };
    // Collect + sort for deterministic archives (read_dir order is unspecified).
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();

    let mut any = false;
    for path in paths {
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            if append_tree(builder, root, &path) {
                any = true;
            }
        } else if meta.is_file() {
            let name = rel_from(root, &path);
            if !name.is_empty() && builder.append_path_with_name(&path, &name).is_ok() {
                any = true;
            }
        }
        // symlinks / special files are intentionally skipped
    }
    any
}

/// Join a workspace-relative repo `prefix` with a repo-relative path.
pub(super) fn join_prefix(prefix: &str, rel: &str) -> String {
    if prefix.is_empty() {
        rel.to_string()
    } else {
        format!("{prefix}/{rel}")
    }
}

/// Append one repo's working tree + `.git` to `builder`, entries named at their
/// workspace-relative path (`<prefix>/…`). Returns whether anything was added.
///
/// The working-tree file set is `git ls-files -c -o --exclude-standard` —
/// tracked plus untracked-non-ignored — so ignored artifacts (`node_modules`,
/// `dist`, `target`, …) are excluded. The `.git` directory is then appended
/// wholesale (it is never gitignored), making each repo a *complete* repo on
/// restore: exact HEAD, index, refs and dirty working tree.
fn append_repo<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    repo_dir: &Path,
    prefix: &str,
) -> bool {
    let output = match std::process::Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["ls-files", "-c", "-o", "--exclude-standard", "-z"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return false,
    };

    let mut any = false;
    for rel in output.stdout.split(|b| *b == 0) {
        if rel.is_empty() {
            continue;
        }
        let Ok(rel) = std::str::from_utf8(rel) else {
            continue;
        };
        let full = repo_dir.join(rel);
        let name = join_prefix(prefix, rel);
        // `ls-files` lists paths, not on-disk state, so skip tracked-but-deleted
        // paths and directories. A symlink is stored as a link entry: it is
        // tracked, so dropping it restores a tree where git reports it deleted.
        let stored = match std::fs::symlink_metadata(&full) {
            Ok(m) if m.is_file() => builder.append_path_with_name(&full, &name).is_ok(),
            Ok(m) if m.is_symlink() => append_symlink(builder, &name, &full),
            _ => continue,
        };
        if stored {
            any = true;
        }
    }

    // Append the `.git` dir so the archived repo is self-contained. Best-effort:
    // if it fails, the archive degrades to a working-tree-only snapshot that
    // restore-on-start rejects, falling back to clone + overlay.
    let git_dir = repo_dir.join(".git");
    if git_dir.is_dir()
        && builder
            .append_dir_all(join_prefix(prefix, ".git"), &git_dir)
            .is_ok()
    {
        any = true;
    }

    any
}

/// Append `path` as a tar symlink entry carrying its target, so the link is
/// stored without being read through. `read_link` never follows it, so this
/// cannot loop or reach outside the archive root.
fn append_symlink<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    name: &str,
    path: &Path,
) -> bool {
    let Ok(target) = std::fs::read_link(path) else {
        return false;
    };
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_mode(0o777);
    builder.append_link(&mut header, name, target).is_ok()
}

/// Build an in-memory **gzipped** tar of all data under `workspace_dir` (git
/// repos enumerated with gitignore honored, everything else included verbatim),
/// or `None` when there's nothing to archive. Entries are stored at their
/// workspace-relative path so the archive untars location-independently onto
/// `workspace_dir`. Runs on a blocking thread (blocking `git` subprocess + `tar`
/// + `std::fs` + deflate).
///
/// Tar entries stream straight into the encoder, so the uncompressed archive is
/// never held in memory — only the compressed result is. Source trees compress
/// several-fold, which shrinks both the S3 transfer and the heap this allocates
/// inside a sandbox whose memory pressure is what makes the snapshot matter.
async fn build_tar(workspace_dir: PathBuf) -> Option<Vec<u8>> {
    tokio::task::spawn_blocking(move || {
        if !workspace_dir.is_dir() {
            return None;
        }
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = tar::Builder::new(encoder);
        if !append_tree(&mut builder, &workspace_dir, &workspace_dir) {
            return None;
        }
        builder.into_inner().ok()?.finish().ok()
    })
    .await
    .ok()
    .flatten()
}

/// Return the plain tar bytes for an archive that may or may not be gzipped.
///
/// Archives are written gzipped; a stored archive without the gzip magic is
/// passed through untouched so it still restores. Runs inside the caller's
/// blocking context.
fn decompress_if_gzip(bytes: Vec<u8>) -> std::io::Result<Vec<u8>> {
    if !bytes.starts_with(&GZIP_MAGIC) {
        return Ok(bytes);
    }
    let mut decoded = Vec::new();
    GzDecoder::new(std::io::Cursor::new(bytes)).read_to_end(&mut decoded)?;
    Ok(decoded)
}

/// Fingerprint the workspace's snapshot content without reading file bodies.
///
/// Walks the exact file set [`build_tar`] would archive — each repo's
/// tracked/untracked-non-ignored working tree plus its whole `.git`, and every
/// loose file outside a repo — and hashes `(workspace-relative path, len,
/// mtime)` for each. `.gitignore`d artifacts inside repos are excluded, so build
/// churn (`node_modules`, `target`, …) does not move the fingerprint. `None`
/// when there is nothing to archive. Runs on a blocking thread (`git` subprocess
/// + `std::fs`).
async fn workspace_fingerprint(workspace_dir: PathBuf) -> Option<u64> {
    tokio::task::spawn_blocking(move || {
        if !workspace_dir.is_dir() {
            return None;
        }
        let mut entries: Vec<(String, u64, u128)> = Vec::new();
        fingerprint_tree(&mut entries, &workspace_dir, &workspace_dir);
        if entries.is_empty() {
            return None;
        }
        entries.sort();
        let mut hasher = DefaultHasher::new();
        entries.hash(&mut hasher);
        Some(hasher.finish())
    })
    .await
    .ok()
    .flatten()
}

/// Metadata twin of [`append_tree`]: a git repo root is enumerated via
/// [`fingerprint_repo`] and not descended into; every other file contributes its
/// metadata and every other directory is walked. Symlinks/special files are
/// skipped, matching the tar walk.
fn fingerprint_tree(entries: &mut Vec<(String, u64, u128)>, root: &Path, dir: &Path) {
    if dir.join(".git").is_dir() {
        fingerprint_repo(entries, dir, &rel_from(root, dir));
        return;
    }
    let Ok(dir_entries) = std::fs::read_dir(dir) else {
        return;
    };
    for path in dir_entries.flatten().map(|e| e.path()) {
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            fingerprint_tree(entries, root, &path);
        } else if meta.is_file() {
            let name = rel_from(root, &path);
            if !name.is_empty() {
                entries.push((name, meta.len(), mtime_nanos(&meta)));
            }
        }
    }
}

/// Metadata twin of [`append_repo`]: the working-tree file set from
/// `git ls-files -c -o --exclude-standard` plus the whole `.git` directory,
/// contributing `(len, mtime)` for each file — and each symlink, which the tar
/// walk stores too — instead of its bytes.
fn fingerprint_repo(entries: &mut Vec<(String, u64, u128)>, repo_dir: &Path, prefix: &str) {
    if let Ok(output) = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_dir)
        .args(["ls-files", "-c", "-o", "--exclude-standard", "-z"])
        .output()
    {
        if output.status.success() {
            for rel in output.stdout.split(|b| *b == 0) {
                if rel.is_empty() {
                    continue;
                }
                let Ok(rel) = std::str::from_utf8(rel) else {
                    continue;
                };
                let full = repo_dir.join(rel);
                if let Ok(meta) = std::fs::symlink_metadata(&full) {
                    if meta.is_file() || meta.is_symlink() {
                        entries.push((join_prefix(prefix, rel), meta.len(), mtime_nanos(&meta)));
                    }
                }
            }
        }
    }
    // `.git` is never gitignored and its churn (commits, ref/index updates) is
    // exactly the state a resuming pod must reproduce, so walk it wholesale.
    fingerprint_dir(
        entries,
        &join_prefix(prefix, ".git"),
        &repo_dir.join(".git"),
    );
}

/// Recursively contribute `(name, len, mtime)` for every regular file under
/// `dir`, keyed at `<name_prefix>/<path-below-dir>`. Symlinks/special files are
/// skipped. Used for a repo's `.git`, which has no gitignore to honor.
fn fingerprint_dir(entries: &mut Vec<(String, u64, u128)>, name_prefix: &str, dir: &Path) {
    let Ok(dir_entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in dir_entries.flatten() {
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let name = format!("{name_prefix}/{file_name}");
        if meta.is_dir() {
            fingerprint_dir(entries, &name, &path);
        } else if meta.is_file() {
            entries.push((name, meta.len(), mtime_nanos(&meta)));
        }
    }
}

/// A file's mtime as nanoseconds since the Unix epoch, or `0` when unavailable —
/// a stable-enough change signal for the periodic fingerprint.
fn mtime_nanos(meta: &std::fs::Metadata) -> u128 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos())
}

/// Extract a repo-set archive, choosing the extraction root by archive shape so
/// supported snapshots restore correctly. Returns `Ok(true)` when the
/// primary repo (with its `.git`) was restored (caller skips the clone), or
/// `Ok(false)` — extracting nothing — when the archive is a
/// working-tree-only snapshot, so the caller clones and overlays instead.
///
/// Archive shapes:
/// - **multi-repo** (primary at `<primary_prefix>/.git`, no top-level
///   `.git`): unpack onto `workspace_dir`.
/// - **primary at workspace root** (`primary_prefix == ""`, top-level `.git`):
///   unpack onto `workspace_dir` (which equals `repo_dir`).
/// - **single-repo, repo-relative** (top-level `.git`, prefixed primary):
///   unpack onto `repo_dir`.
/// - **working-tree-only** (no `.git` anywhere): decline (`Ok(false)`).
///
/// The `tar` crate rejects path-traversal (`..`) entries by default.
async fn extract_workspace(
    bytes: Vec<u8>,
    workspace_dir: PathBuf,
    repo_dir: PathBuf,
    primary_prefix: String,
) -> std::io::Result<bool> {
    tokio::task::spawn_blocking(move || {
        let bytes = decompress_if_gzip(bytes)?;
        let sub_marker = if primary_prefix.is_empty() {
            String::new()
        } else {
            format!("{primary_prefix}/.git")
        };
        let sub_marker_slash = format!("{sub_marker}/");

        // Probe once to classify the archive without extracting. A multi-repo
        // archive is decided by `sub`; a repo-relative archive is decided by
        // `top` and requires scanning to the end to confirm `sub` stays false,
        // so early-break only on the deciding signal.
        let (top_level_git, primary_subdir_git) = {
            let mut archive = tar::Archive::new(std::io::Cursor::new(&bytes));
            let mut top = false;
            let mut sub = false;
            for entry in archive.entries()?.flatten() {
                if let Ok(path) = entry.path() {
                    let p = path.to_string_lossy();
                    if p == ".git" || p.starts_with(".git/") {
                        top = true;
                    }
                    if !sub_marker.is_empty()
                        && (p == sub_marker.as_str() || p.starts_with(sub_marker_slash.as_str()))
                    {
                        sub = true;
                    }
                }
                let done = if sub_marker.is_empty() { top } else { sub };
                if done {
                    break;
                }
            }
            (top, sub)
        };

        // Multi-repo shape: primary lives under its subdir.
        if primary_subdir_git {
            let mut archive = tar::Archive::new(std::io::Cursor::new(&bytes));
            archive.unpack(&workspace_dir)?;
            return Ok(true);
        }
        // Primary is the workspace root itself.
        if primary_prefix.is_empty() && top_level_git {
            let mut archive = tar::Archive::new(std::io::Cursor::new(&bytes));
            archive.unpack(&workspace_dir)?;
            return Ok(true);
        }
        // Single-repo, repo-relative archive meant for repo_dir.
        if top_level_git {
            let mut archive = tar::Archive::new(std::io::Cursor::new(&bytes));
            archive.unpack(&repo_dir)?;
            return Ok(true);
        }
        // Working-tree-only archive: no primary .git — caller clones.
        Ok(false)
    })
    .await
    .map_err(std::io::Error::other)?
}

/// Untar `bytes` over `dir` on a blocking thread, overlaying onto the existing
/// working tree (existing files overwritten, new files added). The `tar` crate
/// rejects path-traversal (`..`) entries by default.
async fn extract_tar(bytes: Vec<u8>, dir: PathBuf) -> std::io::Result<()> {
    tokio::task::spawn_blocking(move || {
        let bytes = decompress_if_gzip(bytes)?;
        let mut archive = tar::Archive::new(std::io::Cursor::new(bytes));
        archive.unpack(&dir)
    })
    .await
    .map_err(std::io::Error::other)?
}

/// Best-effort removal of everything inside `dir` (but not `dir` itself), so a
/// failed restore leaves an empty directory for the fallback clone. Failures are
/// logged, never propagated.
pub(super) async fn clear_dir_contents(dir: &std::path::Path) {
    let dir = dir.to_path_buf();
    let result = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            let meta = std::fs::symlink_metadata(&path)?;
            if meta.is_dir() {
                std::fs::remove_dir_all(&path)?;
            } else {
                std::fs::remove_file(&path)?;
            }
        }
        Ok(())
    })
    .await;
    match result {
        Ok(Ok(())) => {}
        Ok(Err(e)) => warn!(error = %e, "Failed to clear workspace after failed restore"),
        Err(e) => warn!(error = %e, "clear_dir_contents task panicked"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use sandbox_proto::{
        DownloadUploadResponse, GetCredentialsResponse, GetWorkspaceSyncUrlsResponse,
        InterruptedTurn, PendingAgentDelivery, PublishEventRequest,
    };

    /// Counts the only call an upload makes before it touches the network, and
    /// then declines — so a test can tell "attempted an upload" from "skipped"
    /// without standing up an S3 endpoint.
    #[derive(Default)]
    struct CountingBackend {
        url_requests: AtomicUsize,
        gate: Option<UploadGate>,
        /// Mint URLs pointing at a closed port instead of declining, so a
        /// transfer is reached and then fails.
        mint_urls: bool,
    }

    /// Turns the URL mint into a valve: it announces each entry on `started` and
    /// then parks until the test hands out a permit, so an upload can be held
    /// in flight while the test drives more turns at the tracker.
    struct UploadGate {
        started: tokio::sync::mpsc::UnboundedSender<()>,
        release: tokio::sync::Semaphore,
    }

    impl CountingBackend {
        fn attempts(&self) -> usize {
            self.url_requests.load(Ordering::SeqCst)
        }

        /// A backend that blocks inside every upload, plus the receiver that
        /// reports each one starting.
        fn gated() -> (Arc<Self>, tokio::sync::mpsc::UnboundedReceiver<()>) {
            let (started, rx) = tokio::sync::mpsc::unbounded_channel();
            let backend = Arc::new(Self {
                gate: Some(UploadGate {
                    started,
                    release: tokio::sync::Semaphore::new(0),
                }),
                ..Self::default()
            });
            (backend, rx)
        }

        /// Let `n` blocked uploads through.
        fn release(&self, n: usize) {
            self.gate.as_ref().unwrap().release.add_permits(n);
        }

        /// A backend whose URLs resolve but whose transfers cannot land.
        fn minting() -> Self {
            Self {
                mint_urls: true,
                ..Self::default()
            }
        }
    }

    #[async_trait]
    impl BackendClient for CountingBackend {
        async fn publish_event(&self, _req: PublishEventRequest) -> Result<(), tonic::Status> {
            Ok(())
        }
        async fn get_pending_messages(
            &self,
            _task_id: &str,
        ) -> Result<Vec<PendingAgentDelivery>, tonic::Status> {
            Ok(vec![])
        }
        async fn acknowledge_messages(
            &self,
            _task_id: &str,
            _uuids: Vec<String>,
            _session_id: &str,
            _run_id: &str,
            _answered_prompt: Option<&str>,
        ) -> Result<(), tonic::Status> {
            Ok(())
        }
        async fn get_credentials(
            &self,
            _task_id: &str,
        ) -> Result<GetCredentialsResponse, tonic::Status> {
            Err(tonic::Status::unimplemented("counting"))
        }
        async fn get_system_prompt(&self, _task_id: &str) -> Result<String, tonic::Status> {
            Err(tonic::Status::unimplemented("counting"))
        }
        async fn update_credential(
            &self,
            _task_id: &str,
            _key: &str,
            _value: &str,
        ) -> Result<(), tonic::Status> {
            Ok(())
        }
        async fn download_upload(
            &self,
            _task_id: &str,
            _file_id: &str,
        ) -> Result<DownloadUploadResponse, tonic::Status> {
            Err(tonic::Status::unimplemented("counting"))
        }
        async fn recover_interrupted_run(
            &self,
            _task_id: &str,
        ) -> Result<Option<InterruptedTurn>, tonic::Status> {
            Ok(None)
        }
        async fn upload_transcript(
            &self,
            _task_id: &str,
            _tar_data: Vec<u8>,
        ) -> Result<(), tonic::Status> {
            Ok(())
        }
        async fn download_transcript(&self, _task_id: &str) -> Result<Vec<u8>, tonic::Status> {
            Ok(Vec::new())
        }
        async fn get_workspace_sync_urls(
            &self,
            _task_id: &str,
        ) -> Result<GetWorkspaceSyncUrlsResponse, tonic::Status> {
            self.url_requests.fetch_add(1, Ordering::SeqCst);
            if let Some(gate) = &self.gate {
                let _ = gate.started.send(());
                let _permit = gate.release.acquire().await.unwrap();
            }
            if self.mint_urls {
                // Port 1 is bound by nothing, so every transfer is refused
                // without leaving the host.
                return Ok(GetWorkspaceSyncUrlsResponse {
                    put_url: "http://127.0.0.1:1/workspace.tar".to_string(),
                    get_url: "http://127.0.0.1:1/workspace.tar".to_string(),
                    expires_in_secs: 900,
                    manifest_put_url: "http://127.0.0.1:1/manifest.json.gz".to_string(),
                    manifest_get_url: "http://127.0.0.1:1/manifest.json.gz".to_string(),
                });
            }
            Err(tonic::Status::failed_precondition("no S3 in tests"))
        }
    }

    /// A config whose workspace is `dir`, with the two storage switches set.
    fn sync_config(dir: &std::path::Path, sync: bool, restore_if_empty: bool) -> Config {
        let mut config = crate::config::test_support::minimal_config();
        config.workspace_dir = dir.to_string_lossy().into_owned();
        config.repo_dir = dir.join("repo").to_string_lossy().into_owned();
        config.workspace_sync_enabled = sync;
        config.workspace_restore_if_empty = restore_if_empty;
        config
    }

    /// The periodic loop exists to bound the loss from a kill that destroys the
    /// only copy — so it runs exactly when the volume is not durable.
    #[test]
    fn the_periodic_loop_runs_only_without_a_durable_volume() {
        let dir = std::path::Path::new("/tmp/never-read");
        assert!(is_periodic_enabled(&sync_config(dir, true, false)));
        assert!(!is_periodic_enabled(&sync_config(dir, true, true)));
        assert!(!is_periodic_enabled(&sync_config(dir, false, false)));
    }

    #[tokio::test]
    async fn a_committed_repo_is_usable() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        std::fs::write(repo.join("a.txt"), "hello").unwrap();
        git_add(&repo, &["a.txt"]);
        git_commit(&repo, "init");

        assert!(primary_repo_is_usable(&repo).await);
    }

    #[tokio::test]
    async fn an_empty_or_broken_repo_is_not_usable() {
        let tmp = tempfile::tempdir().unwrap();

        // Nothing there at all — a freshly created PVC.
        let missing = tmp.path().join("missing");
        assert!(!primary_repo_is_usable(&missing).await);

        // A directory with files but no `.git` — a half-extracted tar.
        let no_git = tmp.path().join("no-git");
        std::fs::create_dir_all(&no_git).unwrap();
        std::fs::write(no_git.join("a.txt"), "hello").unwrap();
        assert!(!primary_repo_is_usable(&no_git).await);

        // A `.git` that exists but cannot answer `rev-parse HEAD` — an init'd
        // repo with no commit, or one corrupted by a kill mid-write.
        let no_head = tmp.path().join("no-head");
        init_repo(&no_head);
        assert!(!primary_repo_is_usable(&no_head).await);
    }

    /// The overlay is owed only when nothing authoritative is already in place.
    /// A durable volume we chose to keep must suppress it exactly like a
    /// freshly-extracted snapshot — otherwise `download_on_start` extracts the
    /// older archive over the live PVC and reverts work that never reached a
    /// snapshot.
    #[test]
    fn only_a_bare_decision_still_owes_the_overlay() {
        assert!(RestoreDecision::RestoredSnapshot.skips_overlay());
        assert!(RestoreDecision::UseExistingVolume.skips_overlay());
        assert!(!RestoreDecision::Nothing.skips_overlay());

        // Only a real extraction counts as a snapshot origin; keeping the volume
        // lost nothing, so it is not reported as a restore.
        assert!(RestoreDecision::RestoredSnapshot.restored_snapshot());
        assert!(!RestoreDecision::UseExistingVolume.restored_snapshot());
        assert!(!RestoreDecision::Nothing.restored_snapshot());
    }

    /// The gate every upload path shares: nothing travels twice.
    #[tokio::test]
    async fn an_unchanged_workspace_is_not_re_uploaded() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        std::fs::write(repo.join("a.txt"), "hello").unwrap();
        git_add(&repo, &["a.txt"]);
        git_commit(&repo, "init");

        // The tar gate specifically: the mirror's baseline is not a fingerprint
        // and is seeded from S3, so it is covered by its own test below.
        let mut config = sync_config(tmp.path(), true, true);
        config.workspace_mirror_enabled = false;
        let backend = CountingBackend::default();
        let tracker = WorkspaceSyncTracker::new();

        // Init's baseline: the tar the clone/restore produced is whole, so the
        // first turn ships nothing.
        tracker.adopt_current(&config, &backend).await;
        upload_after_turn(&config, &backend, &tracker).await;
        assert_eq!(backend.attempts(), 0, "an untouched workspace is skipped");

        // An agent edit moves the fingerprint.
        std::fs::write(repo.join("a.txt"), "edited by the agent").unwrap();
        upload_after_turn(&config, &backend, &tracker).await;
        assert_eq!(backend.attempts(), 1, "a changed workspace is uploaded");

        // The upload failed (no S3 here), so the baseline must not advance —
        // otherwise a transient failure would silently drop the change.
        upload_after_turn(&config, &backend, &tracker).await;
        assert_eq!(
            backend.attempts(),
            2,
            "a failed upload leaves the change pending for the next attempt"
        );
    }

    /// A clone is not a mirror: a tree S3 holds no objects for must not become
    /// the delta base. Adopting one made the first upload ship only the agent's
    /// later edits under a manifest naming every file in the tree, and the next
    /// restore failed on the first object nobody stored, cleared the workspace
    /// and cloned over the work (PR #684, verified on the cluster).
    #[tokio::test]
    async fn a_clone_is_not_adopted_as_the_mirror_baseline() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        std::fs::write(repo.join("a.txt"), "hello").unwrap();
        git_add(&repo, &["a.txt"]);
        git_commit(&repo, "init");

        let config = sync_config(tmp.path(), true, true);
        assert!(config.workspace_mirror_enabled);
        let backend = CountingBackend::default();
        let tracker = WorkspaceSyncTracker::new();

        // This backend mints no URLs, so S3 holds no manifest to adopt.
        tracker.adopt_current(&config, &backend).await;
        assert_eq!(backend.attempts(), 1, "the baseline is read from S3");
        assert!(
            !tracker.has_mirror_baseline().await,
            "a tree S3 has no objects for is not a delta base"
        );

        // With no baseline the untouched clone is itself the delta, so it ships.
        upload_after_turn(&config, &backend, &tracker).await;
        assert_eq!(backend.attempts(), 2, "the clone itself is uploaded");
    }

    /// A mirror that would not restore is not a delta base either. A task
    /// poisoned by a pre-#686 sandbox holds a manifest naming objects S3 never
    /// stored; adopting it again leaves every unchanged file matching the
    /// manifest, so the missing objects are never re-uploaded and the task can
    /// only ever restore what it already has.
    #[tokio::test]
    async fn a_mirror_that_would_not_restore_is_not_adopted_as_the_baseline() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        std::fs::write(repo.join("a.txt"), "hello").unwrap();
        git_add(&repo, &["a.txt"]);
        git_commit(&repo, "init");

        let config = sync_config(tmp.path(), true, true);
        assert!(config.workspace_mirror_enabled);
        let backend = CountingBackend::default();
        let tracker = WorkspaceSyncTracker::new();

        tracker.mark_mirror_unusable();
        tracker.adopt_current(&config, &backend).await;
        assert_eq!(
            backend.attempts(),
            0,
            "a manifest that failed to restore is not re-fetched"
        );
        assert!(
            !tracker.has_mirror_baseline().await,
            "a mirror we could not restore from is not a delta base"
        );

        // The whole tree is the delta, so the next upload ships what S3 lacks.
        upload_after_turn(&config, &backend, &tracker).await;
        assert_eq!(backend.attempts(), 1, "the whole tree is uploaded");
    }

    /// A mirror that fails still owes the task a durable copy. Task
    /// `5f0f1eaf` ran a whole session against a mirror failing every object
    /// with `411 Length Required`; because a failed transfer returned success,
    /// no tar was written either, and each restart found no copy of any format
    /// and cloned over the work.
    #[tokio::test]
    async fn a_failed_mirror_falls_back_to_the_tar() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        std::fs::write(repo.join("a.txt"), "hello").unwrap();
        git_add(&repo, &["a.txt"]);
        git_commit(&repo, "init");

        let config = sync_config(tmp.path(), true, true);
        assert!(config.workspace_mirror_enabled);
        let backend = CountingBackend::minting();
        let tracker = WorkspaceSyncTracker::new();

        // The mirror mints its URLs and fails the transfer; the tar then mints
        // its own, so two mints mean both formats were attempted.
        upload_after_turn(&config, &backend, &tracker).await;
        assert_eq!(
            backend.attempts(),
            2,
            "a mirror that stored nothing is followed by a tar"
        );

        // Neither format landed, so nothing advanced and the next turn retries.
        assert!(!tracker.has_mirror_baseline().await);
        assert_eq!(tracker.baseline().await, None);
    }

    /// The turn-boundary upload is gated on the S3 switch, not just on the
    /// fingerprint: a sandbox with no snapshot configured must not even ask for
    /// URLs.
    #[tokio::test]
    async fn a_sandbox_without_the_s3_switch_never_uploads() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        std::fs::write(repo.join("a.txt"), "hello").unwrap();
        git_add(&repo, &["a.txt"]);
        git_commit(&repo, "init");

        let config = sync_config(tmp.path(), false, false);
        let backend = CountingBackend::default();
        let tracker = WorkspaceSyncTracker::new();

        upload_after_turn(&config, &backend, &tracker).await;
        assert_eq!(backend.attempts(), 0);
    }

    /// The turn-boundary upload must not hold up the loop that spawned it, and
    /// the turns that arrive while it runs must not each get their own transfer.
    ///
    /// Both halves are the same incident: a mirror that failed on zero-byte
    /// files made every upload burn its full retry budget, and because the
    /// coordinator awaited it, an already-queued wake sat unread for four
    /// minutes (task `5f0f1eaf`).
    #[tokio::test]
    async fn turn_uploads_run_off_the_caller_and_coalesce() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        std::fs::write(repo.join("a.txt"), "hello").unwrap();
        git_add(&repo, &["a.txt"]);
        git_commit(&repo, "init");

        // The tar path: the mirror has its own transport and this is about
        // scheduling, not about what travels.
        let mut config = sync_config(tmp.path(), true, true);
        config.workspace_mirror_enabled = false;
        let config = Arc::new(config);
        let (backend, mut started) = CountingBackend::gated();
        let tracker = WorkspaceSyncTracker::new();

        // First boundary. Returning at all is the point: the upload it starts
        // never finishes on its own.
        spawn_after_turn(config.clone(), backend.clone(), tracker.clone());
        started.recv().await.expect("the first upload started");

        // Three more turns land while that one is stuck. One queues behind it;
        // the other two fold into that one.
        for _ in 0..3 {
            spawn_after_turn(config.clone(), backend.clone(), tracker.clone());
        }
        assert_eq!(
            started.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty),
            "no second transfer starts while the first holds the gate"
        );

        backend.release(8);
        started.recv().await.expect("the queued upload runs");
        assert!(
            tokio::time::timeout(Duration::from_millis(250), started.recv())
                .await
                .is_err(),
            "the three turns behind the first upload share one, not three"
        );
        assert_eq!(backend.attempts(), 2);
    }

    /// A sandbox with no snapshot configured spawns nothing to skip.
    #[tokio::test]
    async fn a_sandbox_without_the_s3_switch_spawns_no_upload() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Arc::new(sync_config(tmp.path(), false, false));
        let (backend, mut started) = CountingBackend::gated();
        let tracker = WorkspaceSyncTracker::new();

        spawn_after_turn(config, backend.clone(), tracker);
        assert!(
            tokio::time::timeout(Duration::from_millis(250), started.recv())
                .await
                .is_err(),
            "the switch is off, so no upload is attempted"
        );
        assert_eq!(backend.attempts(), 0);
    }

    /// Init a git repo at `dir` with one committed file so `git ls-files` works.
    fn init_repo(dir: &std::path::Path) {
        std::fs::create_dir_all(dir).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t.co"],
            vec!["config", "user.name", "t"],
        ] {
            std::process::Command::new("git")
                .arg("-C")
                .arg(dir)
                .args(&args)
                .output()
                .unwrap();
        }
    }

    fn git_add(dir: &std::path::Path, paths: &[&str]) {
        let mut args = vec!["add"];
        args.extend_from_slice(paths);
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(&args)
            .output()
            .unwrap();
    }

    fn git_commit(dir: &std::path::Path, message: &str) {
        std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["commit", "-q", "-m", message])
            .output()
            .unwrap();
    }

    #[test]
    fn read_interval_defaults_and_disables() {
        std::env::remove_var("WORKSPACE_SYNC_INTERVAL_SECS");
        assert_eq!(
            read_interval_from_env(),
            Some(Duration::from_secs(DEFAULT_INTERVAL_SECS))
        );

        std::env::set_var("WORKSPACE_SYNC_INTERVAL_SECS", "0");
        assert_eq!(read_interval_from_env(), None);

        std::env::set_var("WORKSPACE_SYNC_INTERVAL_SECS", "30");
        assert_eq!(read_interval_from_env(), Some(Duration::from_secs(30)));

        std::env::set_var("WORKSPACE_SYNC_INTERVAL_SECS", "garbage");
        assert_eq!(
            read_interval_from_env(),
            Some(Duration::from_secs(DEFAULT_INTERVAL_SECS))
        );

        std::env::remove_var("WORKSPACE_SYNC_INTERVAL_SECS");
    }

    #[tokio::test]
    async fn fingerprint_tracks_snapshot_content_and_ignores_build_churn() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path().join("workspace");
        tokio::fs::create_dir_all(&ws).await.unwrap();

        let repo = ws.join("repo");
        init_repo(&repo);
        tokio::fs::write(repo.join(".gitignore"), b"ignored/\n")
            .await
            .unwrap();
        tokio::fs::write(repo.join("main.rs"), b"fn main() {}\n")
            .await
            .unwrap();
        git_add(&repo, &["main.rs", ".gitignore"]);
        git_commit(&repo, "init");
        tokio::fs::create_dir_all(repo.join("ignored"))
            .await
            .unwrap();
        tokio::fs::write(repo.join("ignored").join("build.log"), b"x\n")
            .await
            .unwrap();

        let base = workspace_fingerprint(ws.clone())
            .await
            .expect("workspace has content");
        // Recomputing without changes is stable.
        assert_eq!(base, workspace_fingerprint(ws.clone()).await.unwrap());

        // Churning a gitignored artifact does not move the fingerprint.
        tokio::fs::write(repo.join("ignored").join("build.log"), b"xxxxxxxx\n")
            .await
            .unwrap();
        assert_eq!(base, workspace_fingerprint(ws.clone()).await.unwrap());

        // Editing a tracked file moves it.
        tokio::fs::write(repo.join("main.rs"), b"fn main() { work(); }\n")
            .await
            .unwrap();
        let after_edit = workspace_fingerprint(ws.clone()).await.unwrap();
        assert_ne!(base, after_edit);

        // A new commit (which rewrites `.git`) moves it again.
        git_add(&repo, &["main.rs"]);
        git_commit(&repo, "work");
        let after_commit = workspace_fingerprint(ws.clone()).await.unwrap();
        assert_ne!(after_edit, after_commit);

        // A loose file outside any repo moves it, too.
        tokio::fs::write(ws.join("notes.txt"), b"scratch\n")
            .await
            .unwrap();
        assert_ne!(
            after_commit,
            workspace_fingerprint(ws.clone()).await.unwrap()
        );
    }

    #[tokio::test]
    async fn archive_is_gzipped_and_smaller_than_the_plain_tar() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        // Compressible, repetitive source content.
        tokio::fs::write(repo.join("main.rs"), "fn main() {}\n".repeat(500))
            .await
            .unwrap();
        git_add(&repo, &["main.rs"]);

        let archive = build_tar(repo.clone()).await.expect("archive built");
        assert!(
            archive.starts_with(&GZIP_MAGIC),
            "archive must be gzipped for the S3 transfer"
        );

        let plain = decompress_if_gzip(archive.clone()).unwrap();
        assert!(
            archive.len() < plain.len(),
            "gzip must shrink the payload: {} compressed vs {} plain",
            archive.len(),
            plain.len()
        );

        // And it still restores to the same content.
        let dst = tmp.path().join("restored");
        tokio::fs::create_dir_all(&dst).await.unwrap();
        extract_tar(archive, dst.clone()).await.unwrap();
        assert_eq!(
            tokio::fs::read_to_string(dst.join("main.rs"))
                .await
                .unwrap(),
            "fn main() {}\n".repeat(500)
        );
    }

    #[tokio::test]
    async fn an_uncompressed_archive_still_restores() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path().join("src-repo");
        init_repo(&repo);
        tokio::fs::write(repo.join("a.txt"), b"hi\n").await.unwrap();
        git_add(&repo, &["a.txt"]);

        // A plain (non-gzipped) tar of the repo at the archive root.
        let plain_tar = tokio::task::spawn_blocking({
            let repo = repo.clone();
            move || {
                let mut builder = tar::Builder::new(Vec::new());
                builder.append_dir_all(".", &repo).unwrap();
                builder.into_inner().unwrap()
            }
        })
        .await
        .unwrap();
        assert!(!plain_tar.starts_with(&GZIP_MAGIC));

        let ws = tmp.path().join("ws");
        let repo_dir = ws.join("primary");
        tokio::fs::create_dir_all(&repo_dir).await.unwrap();
        let restored = extract_workspace(
            plain_tar,
            ws.clone(),
            repo_dir.clone(),
            "primary".to_string(),
        )
        .await
        .unwrap();

        assert!(
            restored,
            "an uncompressed snapshot must still be restorable"
        );
        assert!(repo_dir.join("a.txt").exists());
        assert!(repo_dir.join(".git").is_dir());
    }

    #[tokio::test]
    async fn fingerprint_is_none_for_empty_or_missing_workspace() {
        let tmp = tempfile::TempDir::new().unwrap();
        assert!(
            workspace_fingerprint(tmp.path().join("nope"))
                .await
                .is_none()
        );
        let empty = tmp.path().join("empty");
        tokio::fs::create_dir_all(&empty).await.unwrap();
        assert!(workspace_fingerprint(empty).await.is_none());
    }

    #[test]
    fn primary_prefix_derives_relative_repo_name() {
        assert_eq!(primary_prefix("/ws", "/ws/soap-tm"), "soap-tm");
        assert_eq!(primary_prefix("/ws", "/ws"), "");
        // repo_dir not under workspace_dir → empty (treated as root).
        assert_eq!(primary_prefix("/ws", "/other/repo"), "");
    }

    #[tokio::test]
    async fn tar_round_trip_respects_gitignore() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path().join("repo");
        init_repo(&ws);

        tokio::fs::write(ws.join(".gitignore"), b"ignored/\n")
            .await
            .unwrap();
        tokio::fs::write(ws.join("src.txt"), b"tracked\n")
            .await
            .unwrap();
        tokio::fs::write(ws.join("untracked.txt"), b"untracked\n")
            .await
            .unwrap();
        tokio::fs::create_dir_all(ws.join("ignored")).await.unwrap();
        tokio::fs::write(ws.join("ignored").join("skip.txt"), b"nope\n")
            .await
            .unwrap();
        git_add(&ws, &["src.txt", ".gitignore"]);

        // `ws` itself is a repo → build_tar snapshots it at the archive root.
        let tar = build_tar(ws.clone()).await.expect("tar built");
        assert!(!tar.is_empty());

        let dst = tmp.path().join("restored");
        tokio::fs::create_dir_all(&dst).await.unwrap();
        extract_tar(tar, dst.clone()).await.unwrap();

        // Tracked + untracked-non-ignored are present; the ignored path is not.
        assert!(dst.join("src.txt").exists());
        assert!(dst.join("untracked.txt").exists());
        assert!(!dst.join("ignored").join("skip.txt").exists());
        // The archive carries `.git`, so the restored tree is a real repo.
        assert!(dst.join(".git").is_dir());
    }

    #[tokio::test]
    async fn snapshot_captures_all_data_and_honors_repo_gitignore() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path().join("workspace");
        tokio::fs::create_dir_all(&ws).await.unwrap();

        // Primary repo (a subdir) with a gitignored build artifact + a sibling
        // repo the agent "cloned".
        let primary = ws.join("soap-tm");
        init_repo(&primary);
        tokio::fs::write(primary.join(".gitignore"), b"target/\n")
            .await
            .unwrap();
        tokio::fs::write(primary.join("main.rs"), b"fn main() {}\n")
            .await
            .unwrap();
        tokio::fs::create_dir_all(primary.join("target"))
            .await
            .unwrap();
        tokio::fs::write(primary.join("target").join("app.jar"), b"BINARY\n")
            .await
            .unwrap();
        git_add(&primary, &["main.rs", ".gitignore"]);

        let sibling = ws.join("config-default");
        init_repo(&sibling);
        tokio::fs::write(sibling.join("scenarios.yaml"), b"cases: []\n")
            .await
            .unwrap();
        git_add(&sibling, &["scenarios.yaml"]);

        // Loose data outside any repo: a top-level file and a non-git directory.
        tokio::fs::write(ws.join("notes.txt"), b"scratch\n")
            .await
            .unwrap();
        tokio::fs::create_dir_all(ws.join("artifacts"))
            .await
            .unwrap();
        tokio::fs::write(ws.join("artifacts").join("build.log"), b"ok\n")
            .await
            .unwrap();

        // Snapshot the whole workspace.
        let tar = build_tar(ws.clone()).await.expect("tar built");

        // Restore onto a fresh empty workspace; primary_prefix is the primary
        // repo's subdir name.
        let dst = tmp.path().join("restored-ws");
        tokio::fs::create_dir_all(&dst).await.unwrap();
        let restored =
            extract_workspace(tar, dst.clone(), dst.join("soap-tm"), "soap-tm".to_string())
                .await
                .unwrap();
        assert!(restored, "primary repo present → skip clone");

        // Both repos restored, each a complete repo.
        assert!(dst.join("soap-tm").join("main.rs").exists());
        assert!(dst.join("soap-tm").join(".git").is_dir());
        assert!(dst.join("config-default").join("scenarios.yaml").exists());
        assert!(dst.join("config-default").join(".git").is_dir());

        // Loose (non-repo) data is captured verbatim.
        assert!(dst.join("notes.txt").exists());
        assert!(dst.join("artifacts").join("build.log").exists());

        // But a gitignored artifact *inside a repo* is excluded.
        assert!(
            !dst.join("soap-tm").join("target").join("app.jar").exists(),
            "gitignored file inside a repo must not be snapshotted"
        );
    }

    /// A repo commits its own agent-config symlinks (`CLAUDE.md -> AGENTS.md`,
    /// `.claude/skills -> ../.agents/skills`). Restoring without them leaves a
    /// tree where git reports them deleted, and the deletion gets committed.
    #[tokio::test]
    async fn a_committed_symlink_survives_a_snapshot_round_trip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path().join("workspace");
        tokio::fs::create_dir_all(&ws).await.unwrap();

        let repo = ws.join("moatless");
        init_repo(&repo);
        tokio::fs::write(repo.join("AGENTS.md"), b"# guidance\n")
            .await
            .unwrap();
        std::os::unix::fs::symlink("AGENTS.md", repo.join("CLAUDE.md")).unwrap();
        git_add(&repo, &["AGENTS.md", "CLAUDE.md"]);
        git_commit(&repo, "add guidance");

        let tar = build_tar(ws.clone()).await.expect("tar built");

        let dst = tmp.path().join("restored-ws");
        tokio::fs::create_dir_all(&dst).await.unwrap();
        let restored = extract_workspace(
            tar,
            dst.clone(),
            dst.join("moatless"),
            "moatless".to_string(),
        )
        .await
        .unwrap();
        assert!(restored, "primary repo present → skip clone");

        let link = dst.join("moatless").join("CLAUDE.md");
        assert!(
            std::fs::symlink_metadata(&link).unwrap().is_symlink(),
            "committed symlink must restore as a symlink"
        );
        assert_eq!(std::fs::read_link(&link).unwrap(), Path::new("AGENTS.md"));

        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dst.join("moatless"))
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        assert!(
            status.stdout.is_empty(),
            "restored repo must be clean, got: {}",
            String::from_utf8_lossy(&status.stdout)
        );
    }

    #[tokio::test]
    async fn repo_relative_archive_restores_to_repo_dir() {
        let tmp = tempfile::TempDir::new().unwrap();

        // Build a repo-relative archive: a single repo tarred at the archive
        // root (top-level `.git`).
        let repo = tmp.path().join("repo-relative-src");
        init_repo(&repo);
        tokio::fs::write(repo.join("a.txt"), b"hi\n").await.unwrap();
        git_add(&repo, &["a.txt"]);
        let repo_relative_tar = build_tar(repo.clone()).await.expect("tar built");

        // Restore with a non-empty primary_prefix (repo lives in a subdir of the
        // workspace) → must unpack onto repo_dir, not workspace_dir.
        let ws = tmp.path().join("ws");
        let repo_dir = ws.join("soap-tm");
        tokio::fs::create_dir_all(&repo_dir).await.unwrap();
        let restored = extract_workspace(
            repo_relative_tar,
            ws.clone(),
            repo_dir.clone(),
            "soap-tm".to_string(),
        )
        .await
        .unwrap();
        assert!(restored);
        assert!(repo_dir.join("a.txt").exists());
        assert!(repo_dir.join(".git").is_dir());
        // Not misplaced at the workspace root.
        assert!(!ws.join("a.txt").exists());
    }

    #[tokio::test]
    async fn working_tree_only_archive_is_declined() {
        // A working-tree-only archive (no `.git`) declines so the caller clones
        // and overlays.
        let mut archive = tar::Builder::new(Vec::new());
        let data = b"working tree\n";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        archive
            .append_data(&mut header, "only-file.txt", &data[..])
            .unwrap();
        let working_tree_tar = archive.into_inner().unwrap();

        let tmp = tempfile::TempDir::new().unwrap();
        let ws = tmp.path().join("ws");
        let repo_dir = ws.join("soap-tm");
        tokio::fs::create_dir_all(&repo_dir).await.unwrap();
        let restored = extract_workspace(
            working_tree_tar,
            ws.clone(),
            repo_dir.clone(),
            "soap-tm".to_string(),
        )
        .await
        .unwrap();
        assert!(!restored);
        assert!(!repo_dir.join("only-file.txt").exists());
        assert!(!ws.join("only-file.txt").exists());
    }

    #[tokio::test]
    async fn rescue_upload_fails_when_there_is_nothing_to_archive() {
        let tmp = tempfile::TempDir::new().unwrap();
        let missing = tmp.path().join("no-such-workspace");

        let err = rescue_upload(missing.to_str().unwrap(), "http://127.0.0.1:1/put")
            .await
            .expect_err("an unarchivable workspace is an error, not an empty success");

        // The caller deletes the volume on Ok, so "nothing to send" must not
        // read as "sent" — and the message has to name the directory, since the
        // rescue pod's log is the only place anyone will look.
        assert!(err.contains("no-such-workspace"), "{err}");
    }

    #[tokio::test]
    async fn rescue_upload_fails_when_the_transfer_does() {
        let tmp = tempfile::TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        init_repo(&repo);
        tokio::fs::write(repo.join("main.rs"), "fn main() {}\n")
            .await
            .unwrap();
        git_add(&repo, &["main.rs"]);

        // Port 1 is reserved and unbound: the PUT cannot land.
        rescue_upload(repo.to_str().unwrap(), "http://127.0.0.1:1/put")
            .await
            .expect_err("a rejected transfer must surface, not be swallowed");
    }

    /// The framing regression behind `411 Length Required`.
    ///
    /// reqwest sends an empty body with neither `Content-Length` nor
    /// `Transfer-Encoding`, and S3 rejects an unframed `PUT`. The mirror writes
    /// one object per file, so a single `.gitkeep` failed every sync of that
    /// workspace — permanently, because the manifest is only written after the
    /// objects land.
    #[tokio::test]
    async fn an_empty_body_is_still_framed_with_a_content_length() {
        let framing = framing_seen_by_a_server(Vec::new()).await;
        assert_eq!(
            framing.as_deref(),
            Some("content-length: 0"),
            "an unframed PUT is what S3 answers with 411"
        );
    }

    #[tokio::test]
    async fn a_non_empty_body_is_framed_with_its_length() {
        let framing = framing_seen_by_a_server(b"hello".to_vec()).await;
        assert_eq!(framing.as_deref(), Some("content-length: 5"));
    }

    /// `http_put` one body at a throwaway socket; report the framing header the
    /// server actually received, lowercased.
    async fn framing_seen_by_a_server(body: Vec<u8>) -> Option<String> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let port = listener.local_addr().expect("addr").port();

        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.expect("accept");
            let mut buf = vec![0u8; 4096];
            let n = sock.read(&mut buf).await.expect("read");
            sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
                .await
                .ok();
            String::from_utf8_lossy(&buf[..n])
                .lines()
                .map(str::to_ascii_lowercase)
                .find(|l| l.starts_with("content-length") || l.starts_with("transfer-encoding"))
        });

        let client = http_client(Duration::from_secs(5)).expect("client");
        http_put(&client, &format!("http://127.0.0.1:{port}/obj"), body)
            .await
            .expect("the throwaway server answers 200");
        server.await.expect("server task")
    }
}
