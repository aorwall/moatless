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
use tracing::{debug, error, info, warn};

use super::restic;
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::state::SandboxState;

/// The `format` label every transfer this module makes is reported under: it
/// writes and reads one gzipped tar. The `restic` repository reports
/// `"restic"`.
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
    /// Whether this transfer moved workspace data. It is what
    /// `SyncWorkspaceResponse.synced` carries, so it answers the wire's
    /// question — not "did any request reach S3". A `restic` snapshot of an
    /// unchanged tree writes a snapshot object and still reports `false`,
    /// because no workspace bytes moved.
    pub synced: bool,
    /// Bytes of workspace data this transfer moved; `0` when nothing did.
    pub bytes: u64,
}

/// Why the workspace reached neither store.
pub(super) enum NotStored {
    /// This backend serves no such store, or the workspace held nothing to
    /// store, so nothing was owed.
    Absent,
    /// Something was owed and did not land, so the caller must offer the tree
    /// again.
    Failed,
}

/// The fingerprint of the last successfully uploaded workspace, shared by every
/// upload path.
///
/// Turn boundaries, the pre-stop RPC, the shutdown flush and the periodic loop
/// all ask the same question — *has anything changed since we last shipped?* —
/// so they share one answer. Without that, each path keeps its own baseline and
/// a turn-boundary upload is immediately followed by an identical periodic one.
///
/// The fingerprint is taken *before* anything is written and stored only after
/// [`WorkspaceSyncTracker::store_current`] reports every enabled format holding
/// the tree, so a change made while the transfer ran is not recorded as shipped:
/// the next attempt uploads again rather than skipping.
#[derive(Clone, Default)]
pub struct WorkspaceSyncTracker {
    last_uploaded: Arc<tokio::sync::Mutex<Option<u64>>>,
    /// Shared by [`restic::upload`], [`restic::restore_on_start`] and
    /// [`restic::forget_and_prune`].
    pub(super) repository_access: Arc<restic::AccessCache>,
    /// Written by [`Self::hold_uploads`], read by [`Self::upload_if_changed`].
    uploads_held: Arc<AtomicBool>,
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
    pub async fn adopt_current(&self, config: &Config) {
        let dir = PathBuf::from(&config.workspace_dir);
        *self.last_uploaded.lock().await = workspace_fingerprint(dir).await;
    }

    /// Refuse every upload for this pod's life.
    ///
    /// Do not call this for a task that merely has no snapshot. It is for a
    /// snapshot that exists and could not be read: that snapshot is then the
    /// only copy of the work, and this pod holds a clone or an older archive
    /// instead, so an upload replaces the copy with a tree that never held it.
    /// Holding costs this pod's own new work, which is the lesser loss — a
    /// restart re-reads the snapshot, and a transient failure clears itself.
    pub fn hold_uploads(&self) {
        self.uploads_held.store(true, Ordering::Relaxed);
    }

    /// The baseline the upload gate compares against. `None` means nothing has
    /// been shipped or adopted, so the next attempt uploads.
    #[cfg(test)]
    pub(crate) async fn baseline(&self) -> Option<u64> {
        *self.last_uploaded.lock().await
    }

    /// Whether every upload is held. See [`Self::hold_uploads`].
    #[cfg(test)]
    pub(crate) fn uploads_are_held(&self) -> bool {
        self.uploads_held.load(Ordering::Relaxed)
    }

    /// Upload the workspace unless it is unchanged since the last successful
    /// upload. Best-effort: a failed transfer leaves the baseline alone so the
    /// next attempt retries.
    async fn upload_if_changed(
        &self,
        config: &Config,
        backend_client: &dyn BackendClient,
    ) -> SyncOutcome {
        if self.uploads_held.load(Ordering::Relaxed) {
            warn!(task_id = %config.task_id, "Upload held: the stored snapshot could not be read on start, and this tree would overwrite it");
            return SyncOutcome::default();
        }

        // One gate for every format, ahead of all of them. A tree that has not
        // moved is already stored by whatever ran last, and a `restic` snapshot
        // of it still walks the workspace, reads the index and writes a
        // snapshot object for no data: 1114 of 3095 snapshots on 2026-09-08 did
        // exactly that. Agent work always moves the fingerprint, so what the
        // gate skips is only a tree init produced and its source still holds.
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

        let stored = self.store_current(config, backend_client).await;
        if stored.complete {
            *last = current;
        }
        stored.outcome
    }

    /// Write the workspace as a `restic` snapshot, falling back to the tar.
    ///
    /// The repository failing holds the baseline back even when the tar stored
    /// the tree, because [`restore_on_start`] reads the repository first and
    /// never compares generations: a repository one tick behind the tar is
    /// still what a restart gets. The cost is that the next tick re-writes what
    /// succeeded, and only while the repository is failing.
    async fn store_current(&self, config: &Config, backend_client: &dyn BackendClient) -> Stored {
        // The repository is incremental and atomic, so it runs first and its
        // outcome is what a caller sees.
        let mut stored = None;
        let mut missed = false;
        match restic::upload(config, backend_client, &self.repository_access).await {
            Ok(outcome) => stored = Some(outcome),
            Err(NotStored::Absent) => {}
            Err(NotStored::Failed) => {
                missed = true;
                debug!(task_id = %config.task_id, "Repository stored nothing; falling back");
            }
        }

        // The repository holds the tree, so a tar would duplicate it. A tar that
        // lands stands in for the repository that missed and does not repair it,
        // so it does not clear `missed`.
        if stored.is_none() {
            match upload_from_config(config, backend_client).await {
                Ok(outcome) => stored = Some(outcome),
                Err(NotStored::Absent) => {}
                Err(NotStored::Failed) => missed = true,
            }
        }

        Stored {
            outcome: stored.unwrap_or_default(),
            complete: !missed,
        }
    }
}

/// What one pass over the formats produced.
struct Stored {
    /// What the caller reports, from the first format that stored the tree.
    outcome: SyncOutcome,
    /// Whether the repository holds this tree. Only `true` may move the upload
    /// baseline.
    complete: bool,
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
pub(super) async fn primary_repo_is_usable(repo_dir: &Path) -> bool {
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
    /// Nothing was restored, and no archive may be overlaid either: a format
    /// holds a snapshot this pod could not read, so the tar is an older copy of
    /// the same workspace rather than a working-tree addition to it. Clone, and
    /// run on the clone alone.
    CloneOnly,
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
        matches!(
            self,
            Self::RestoredSnapshot | Self::UseExistingVolume | Self::CloneOnly
        )
    }
}

/// What the repository's restore concluded, and whether the tar may run.
///
/// Three states, not two. [`Self::Absent`] and [`Self::Failed`] both leave the
/// workspace empty, but only `Absent` may fall through. The tar is a strictly
/// older copy of the same workspace, so descending after a read failure reverts
/// the task to whatever that copy last held — and the tar is written only when
/// the repository stored nothing, so its age is unbounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RestoreAttempt {
    /// The format restored the workspace.
    Restored(RestoreDecision),
    /// The format holds nothing for this task. An older format may run.
    Absent,
    /// The format holds a snapshot this sandbox could not read.
    Failed,
}

/// Clone, and hold every upload, because the repository holds a snapshot this
/// pod could not read.
///
/// The clone is the only tree with a known provenance left: the remote's. It is
/// missing whatever the snapshot held and the remote does not, which is why the
/// hold matters more than the clone — it keeps the snapshot readable by the next
/// pod instead of overwriting it with this one.
///
/// Returns [`RestoreDecision::CloneOnly`], not `Nothing`: `Nothing` still owes
/// [`download_on_start`]'s overlay, which extracts the same rescue tar this walk
/// just refused to descend to.
fn hold_for_unread_snapshot(state: &SandboxState) -> RestoreDecision {
    error!(
        task_id = %state.config.task_id,
        format = restic::FORMAT,
        "Stored workspace snapshot could not be read; cloning and holding every upload so it is not overwritten"
    );
    state.workspace_sync.hold_uploads();
    // The hold lasts the pod's life and silently ends this task's durability,
    // so it is published rather than left in a container log that dies with it.
    let payload = serde_json::json!({
        "format": restic::FORMAT,
        "message": "The stored workspace snapshot could not be read. This pod is running on a fresh clone, and every upload is held so the snapshot is not overwritten.",
    });
    match serde_json::to_string(&payload) {
        Ok(payload_json) => {
            state.events.enqueue(sandbox_proto::PublishEventRequest {
                task_id: state.config.task_id.clone(),
                event_type: sandbox_proto::EventType::Sandbox as i32,
                sub_type: "workspace_uploads_held".to_string(),
                payload_json,
                event_id: String::new(),
                sequence: 0,
                run_id: String::new(),
            });
        }
        Err(e) => warn!(error = %e, "Failed to serialize the upload-hold event"),
    }
    RestoreDecision::CloneOnly
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
/// A repository that failed to read a snapshot it holds instead returns
/// [`RestoreDecision::CloneOnly`], and holds every upload for this pod — see
/// [`hold_for_unread_snapshot`].
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

    // The repository first, and the tar last: the tar is the rescue path and
    // what a task last synced by an older sandbox still has.
    //
    // A repository that fails to read a snapshot it holds stops the walk rather
    // than descending to the tar; see [`RestoreAttempt`].
    let repository = restic::restore_on_start(state).await;
    if let RestoreAttempt::Restored(decision) = repository {
        return decision;
    }
    if repository == RestoreAttempt::Failed {
        return hold_for_unread_snapshot(state);
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
                    Some(&e.to_string()),
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
                    Some(&e.to_string()),
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

/// Store everything under `workspace_dir` in each format this config enables,
/// unless the workspace is unchanged since the last successful upload. No-op
/// when there's nothing to sync. Best-effort: failures (including a disabled
/// feature) are logged, never propagated.
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
/// an S3 transfer. When uploads started failing on zero-byte files, each
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
///
/// This is also the only place the repository is pruned. A prune rewrites the
/// index, which is not work a turn boundary should pay for, and a task being
/// stopped is the one moment there is no next turn to slow down.
pub async fn sync_on_request(state: &SandboxState) -> SyncOutcome {
    if !state.config.workspace_sync_enabled {
        debug!(task_id = %state.config.task_id, "SyncWorkspace requested but workspace sync is off");
        return SyncOutcome::default();
    }
    let outcome = upload(state).await;
    restic::forget_and_prune(
        &state.config,
        state.backend_client.as_ref(),
        &state.workspace_sync.repository_access,
    )
    .await;
    outcome
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

/// Tar the workspace and PUT it, or report that it reached no bucket.
async fn upload_from_config(
    config: &Config,
    backend_client: &dyn BackendClient,
) -> Result<SyncOutcome, NotStored> {
    let task_id = &config.task_id;
    let workspace_dir = PathBuf::from(&config.workspace_dir);

    let Some(tar) = build_tar(workspace_dir).await else {
        debug!(task_id = %task_id, "No workspace files to upload");
        return Err(NotStored::Absent);
    };

    let urls = match backend_client.get_workspace_sync_urls(task_id).await {
        Ok(u) => u,
        Err(e) if is_feature_disabled(&e) => {
            debug!(task_id = %task_id, "Workspace sync unavailable (backend S3 not configured); skipping upload");
            return Err(NotStored::Absent);
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to mint workspace sync URLs (non-fatal)");
            return Err(NotStored::Failed);
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

    if error.is_some() {
        return Err(NotStored::Failed);
    }
    Ok(SyncOutcome {
        synced: true,
        bytes,
    })
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
/// output, or one whose last turn already uploaded ships nothing.
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
pub(super) fn is_feature_disabled(status: &tonic::Status) -> bool {
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
/// presigned host.
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
/// Required`. Setting the header explicitly frames all three cases: an empty
/// body, a small one, and a large one.
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
        .map_err(|e| format!("reading the GET body failed: {e}"))?;
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
mod tests;
