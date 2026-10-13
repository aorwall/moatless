//! Direct-to-S3 persistence of everything under `WORKSPACE_DIR`, enabled by the
//! repo's `s3` storage switch.
//!
//! Inside a git repo the contents come from `git ls-files -c -o
//! --exclude-standard` plus `.git`; outside one everything is included, stored
//! workspace-relative so a tar untars location-independently, letting
//! [`restore_on_start`] skip the clone. The sandbox holds no AWS credentials;
//! [`WorkspaceSyncTracker`] skips a transfer when nothing moved.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant, UNIX_EPOCH};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use moatless_types::http::HttpTimeouts;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

use super::phases::Phases;
use super::{file_set, restic};
use crate::agent::types::AgentStatus;
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::init::InitPhase;
use crate::state::SandboxState;

/// The `format` label every transfer this module makes is reported under: it
/// writes and reads one gzipped tar. The `restic` repository reports
/// `"restic"`.
const FORMAT: &str = "tar";

/// Default periodic-upload interval; `WORKSPACE_SYNC_INTERVAL_SECS` overrides.
///
/// Snapshots between prunes, counted down from `0` so a pod's first one prunes.
///
/// A pod killed by a reclaim, an eviction or a SIGKILL never answers the
/// pre-stop RPC, so its successor is the only thing that can reclaim what it
/// left. Keeps the repository between `KEEP_SNAPSHOTS` and twice that.
const PRUNE_EVERY_SNAPSHOTS: u32 = 50;

/// Bound on the shutdown flush, one step `crate::shutdown` fits in the grace.
pub(crate) const SHUTDOWN_FLUSH_TIMEOUT: Duration = Duration::from_secs(40);

/// Bound on the one gRPC call [`flush_on_shutdown`] makes after its budget is
/// already spent, so reporting the loss cannot itself overrun the grace period.
pub(crate) const TIMEOUT_REPORT_TIMEOUT: Duration = Duration::from_secs(2);

/// The `format` a timed-out shutdown flush reports under, which
/// `record_workspace_sync` buckets as `other`, out of both formats' rates.
const FORMAT_UNKNOWN: &str = "unknown";

/// Leading bytes of a gzip member, used to tell a compressed archive from a
/// plain tar on restore.
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

/// What one upload attempt did, for the caller that has to report it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SyncOutcome {
    /// Whether this transfer moved workspace data — what
    /// `SyncWorkspaceResponse.synced` carries.
    ///
    /// A `restic` snapshot of an unchanged tree still writes a snapshot object
    /// but reports `false`.
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

/// The fingerprint of the last successfully uploaded workspace.
///
/// Shared by every upload path. Taken *before* anything is written and stored
/// only once every enabled format confirms it holds the tree, so a mid-transfer
/// change is not recorded as shipped.
#[derive(Clone, Default)]
pub struct WorkspaceSyncTracker {
    last_uploaded: Arc<tokio::sync::Mutex<Option<u64>>>,
    /// Shared by [`restic::upload`], [`restic::restore_on_start`] and
    /// [`restic::forget_and_prune`].
    pub(super) repository_access: Arc<restic::AccessCache>,
    /// Written by [`Self::hold_uploads`], read by [`Self::upload_if_changed`].
    uploads_held: Arc<AtomicBool>,
    turn_upload: Arc<TurnUpload>,
    /// Snapshots this pod may still write before it prunes. See
    /// [`PRUNE_EVERY_SNAPSHOTS`] for why `0` — the `Default` — is the start.
    snapshots_until_prune: Arc<AtomicU32>,
}

/// Admission control for the turn-boundary upload, run off the agent's main
/// loop ([`spawn_after_turn`]).
///
/// At most one upload runs and at most one waits; it ships the workspace as it
/// is when it starts, so one trailing run covers every turn queued behind it.
#[derive(Default)]
struct TurnUpload {
    /// Held for one upload. Also what makes "one waiting" a real queue rather
    /// than a second concurrent transfer.
    gate: tokio::sync::Mutex<()>,
    /// Set by whoever spawns, cleared only once that task holds the gate.
    ///
    /// Clearing it earlier would let a third turn queue behind one that's still
    /// only one deep — unbounded growth.
    queued: AtomicBool,
}

impl WorkspaceSyncTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adopt the current workspace as the baseline without uploading it.
    ///
    /// Called once init has placed the repos, which already match this
    /// baseline.
    pub async fn adopt_current(&self, config: &Config) {
        let dir = PathBuf::from(&config.workspace_dir);
        *self.last_uploaded.lock().await = workspace_fingerprint(dir).await;
    }

    /// Refuse every upload for this pod's life.
    ///
    /// Only for a snapshot that exists and could not be read, never for a task
    /// with no snapshot: an upload here would overwrite the only copy of the
    /// work with a tree that never held it.
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
        prune: Prune,
        capture: Capture,
        phases: &Arc<Phases>,
    ) -> SyncOutcome {
        if self.uploads_held.load(Ordering::Relaxed) {
            warn!(task_id = %config.task_id, "Upload held: the stored snapshot could not be read on start, and this tree would overwrite it");
            return SyncOutcome::default();
        }

        // One gate for every format, ahead of all of them: a `restic` snapshot
        // of an unmoved tree still walks the workspace, reads the index, and
        // writes a snapshot object for no new data. Agent work always moves
        // the fingerprint, so what this gate skips is only a redundant
        // snapshot whose source tree hasn't changed since the last upload.
        phases.enter("lock");
        let mut last = self.last_uploaded.lock().await;
        phases.enter("fingerprint");
        let current = workspace_fingerprint(PathBuf::from(&config.workspace_dir)).await;
        if current.is_none() {
            debug!(task_id = %config.task_id, "Workspace is empty; nothing to upload");
            return SyncOutcome::default();
        }
        // The shutdown flush snapshots an unchanged tree anyway, so that a
        // missing tag means the flush did not finish and never means it had
        // nothing to do. Costs one empty delta per pod, against telling an
        // agent its intact tree may have lost work.
        if current == *last && !capture.is_shutdown_flush() {
            debug!(task_id = %config.task_id, "Workspace unchanged; skipping upload");
            return SyncOutcome::default();
        }

        let stored = self
            .store_current(config, backend_client, capture, phases)
            .await;
        if stored.complete {
            *last = current;
        }
        // Still under `last`, deliberately: `restic::prune` sets
        // `instant_delete`, so a snapshot writing against the index a prune
        // is rewriting can be left pointing at an already-gone pack. Blocks
        // only another upload in this pod, including the next turn's --
        // `spawn_after_turn` already runs that off the agent's main loop.
        if prune == Prune::WhenDue && stored.snapshotted && self.claim_prune() {
            restic::forget_and_prune(config, backend_client, &self.repository_access).await;
        }
        stored.outcome
    }

    /// Whether this snapshot is the one that prunes, counting down from
    /// [`PRUNE_EVERY_SNAPSHOTS`].
    fn claim_prune(&self) -> bool {
        self.snapshots_until_prune
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |left| {
                Some(if left == 0 {
                    PRUNE_EVERY_SNAPSHOTS
                } else {
                    left - 1
                })
            })
            .is_ok_and(|left| left == 0)
    }

    /// Write the workspace as a `restic` snapshot, falling back to the tar.
    ///
    /// A repository failure holds the baseline back even when the tar stored
    /// the tree: the repository is read first on restart and never compared for
    /// generations, so a stale one is still what the restart gets.
    async fn store_current(
        &self,
        config: &Config,
        backend_client: &dyn BackendClient,
        capture: Capture,
        phases: &Arc<Phases>,
    ) -> Stored {
        // The repository is incremental and atomic, so it runs first and its
        // outcome is what a caller sees.
        let mut stored = None;
        let mut missed = false;
        let mut snapshotted = false;
        let access = &self.repository_access;
        match restic::upload(config, backend_client, access, capture, phases).await {
            Ok(outcome) => {
                snapshotted = true;
                stored = Some(outcome);
            }
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
            phases.enter("tar");
            match upload_from_config(config, backend_client).await {
                Ok(outcome) => stored = Some(outcome),
                Err(NotStored::Absent) => {}
                Err(NotStored::Failed) => missed = true,
            }
        }

        Stored {
            outcome: stored.unwrap_or_default(),
            complete: !missed,
            snapshotted,
        }
    }
}

/// Whether an upload may also prune when the countdown comes due.
///
/// `Never` for the two paths already racing the pod's grace period: the
/// shutdown flush, whose budget is the evidence its report stands on, and the
/// pre-stop RPC, which prunes unconditionally once the upload returns.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Prune {
    WhenDue,
    Never,
}

/// Which upload this is, and for the shutdown flush whether any writer this pod
/// can observe was live when it began.
///
/// `writers_quiet` is half of what the tag claims. The capture earns the other
/// half by finding the tree unchanged across it, which covers a writer admitted
/// after this was read and one in a container this pod cannot see.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Capture {
    Periodic,
    ShutdownFlush { writers_quiet: bool },
}

impl Capture {
    /// Whether this is the shutdown flush, which stores an unchanged tree that
    /// [`WorkspaceSyncTracker::upload_if_changed`] would otherwise skip.
    fn is_shutdown_flush(self) -> bool {
        matches!(self, Self::ShutdownFlush { .. })
    }

    /// Whether this capture may earn the final-tree tag. `restic::snapshot`
    /// writes it only if the tree also holds still from the first byte read to
    /// the last.
    pub(super) fn may_certify_final_tree(self) -> bool {
        matches!(
            self,
            Self::ShutdownFlush {
                writers_quiet: true
            }
        )
    }
}

/// What one pass over the formats produced.
struct Stored {
    /// What the caller reports, from the first format that stored the tree.
    outcome: SyncOutcome,
    /// Whether the repository holds this tree. Only `true` may move the upload
    /// baseline.
    complete: bool,
    /// Whether this pass wrote a `restic` snapshot, so it is one the prune
    /// countdown counts. Not `complete`, which a tar standing in for an absent
    /// repository also satisfies.
    snapshotted: bool,
}

/// Whether the periodic upload loop should run at all.
///
/// Bounds the loss from an involuntary kill that destroys the workspace before
/// any other upload path runs; a durable volume survives that kill, so on the
/// PVC tier the loop is pure overhead.
pub fn is_periodic_enabled(config: &Config) -> bool {
    config.workspace_sync_enabled && !config.workspace_restore_if_empty
}

/// Whether the local volume already holds a usable primary repo.
///
/// Deliberately the same test the agent would fail on: a `.git` that exists
/// *and* answers `git rev-parse HEAD`. Anything else — empty, half-extracted,
/// corrupted — reads as "not usable" and lets the snapshot win.
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
/// Not a bool: collapsing "volume already has the repo" and "nothing restored"
/// into one `false` would let an older archive overlay onto a live durable
/// volume and revert work never in the snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreDecision {
    /// A full snapshot (carrying `.git`) was extracted. Skip the clone, skip
    /// the overlay — the snapshot *is* the working tree.
    RestoredSnapshot {
        /// When the restored snapshot started, so the restart note can date the
        /// tree. `None` when the format carries no timestamp (the rescue tar),
        /// which reads as "unknown", not "current".
        taken_at: Option<chrono::DateTime<chrono::Utc>>,
        /// Whether [`Capture::ShutdownFlush`] wrote this snapshot, which makes
        /// the tree the previous pod's final state. `false` means unknown, not
        /// lost: the rescue tar carries no tag either.
        final_flush: bool,
    },
    /// The durable volume already holds a usable primary repo. Nothing was
    /// fetched; the volume is the live copy. Skip the overlay — it is the older
    /// copy and would revert the volume.
    UseExistingVolume,
    /// Nothing was restored. Clone as usual, then overlay any working-tree
    /// archive.
    Nothing,
    /// Nothing was restored, and no archive may be overlaid: a format holds a
    /// snapshot this pod could not read, so the tar is an older copy, not an
    /// addition. Clone and run on it alone.
    CloneOnly,
}

impl RestoreDecision {
    /// Whether a full snapshot was extracted — the only case that reports
    /// `RepositoryRestored` and counts as a snapshot origin.
    pub fn restored_snapshot(self) -> bool {
        matches!(self, Self::RestoredSnapshot { .. })
    }

    /// When the restored snapshot was taken, for the restart note. `None` both
    /// for a decision that restored nothing and for a format that carries no
    /// timestamp; the note words those the same way, since neither lets an
    /// agent date the tree it is looking at.
    pub fn snapshot_taken_at(self) -> Option<chrono::DateTime<chrono::Utc>> {
        match self {
            Self::RestoredSnapshot { taken_at, .. } => taken_at,
            _ => None,
        }
    }

    /// Whether the shutdown flush wrote the restored snapshot, so the restart
    /// note can tell the agent its tree is intact rather than dating it.
    pub fn restored_final_flush(self) -> bool {
        matches!(
            self,
            Self::RestoredSnapshot {
                final_flush: true,
                ..
            }
        )
    }

    /// Whether the workspace already holds the authoritative copy, so
    /// [`download_on_start`]'s overlay must not run. True for both the snapshot
    /// we just extracted and the durable volume we chose to keep.
    pub fn skips_overlay(self) -> bool {
        matches!(
            self,
            Self::RestoredSnapshot { .. } | Self::UseExistingVolume | Self::CloneOnly
        )
    }
}

/// What the repository's restore concluded, and whether the tar may run.
///
/// [`Self::Absent`] and [`Self::Failed`] both leave the workspace empty, but
/// only `Absent` may fall through: the tar is a strictly older copy, so
/// descending after a read failure reverts the task to whatever it last held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RestoreAttempt {
    /// The format restored the workspace.
    Restored(RestoreDecision),
    /// The format holds nothing for this task. An older format may run.
    Absent,
    /// The format holds a snapshot this sandbox could not read.
    Failed,
}

/// Clone and hold every upload; the repository's snapshot could not be read.
///
/// Returns [`RestoreDecision::CloneOnly`], not `Nothing`: `Nothing` owes
/// [`download_on_start`]'s overlay, which re-extracts the tar this walk refused
/// to descend to.
fn hold_for_unread_snapshot(state: &SandboxState) -> RestoreDecision {
    error!(
        task_id = %state.config.task_id,
        format = restic::FORMAT,
        "Stored workspace snapshot could not be read; cloning and holding every upload so it is not overwritten"
    );
    state.workspace_sync.hold_uploads();
    // The hold lasts the pod's life and silently ends this task's durability,
    // so it is published rather than left in a container log that dies with it.
    publish_workspace_event(
        state,
        "workspace_uploads_held",
        "The stored workspace snapshot could not be read. This pod is running on a fresh clone, and every upload is held so the snapshot is not overwritten.",
    );
    RestoreDecision::CloneOnly
}

/// Report that the restore left out a snapshot object it could not read.
///
/// Says only that one was skipped: its time is inside the object that will not
/// read, so whether the tree is behind what the task last stored is unknown.
pub(super) fn report_skipped_unreadable_snapshot(state: &SandboxState) {
    error!(
        task_id = %state.config.task_id,
        format = restic::FORMAT,
        "Skipped a stored workspace snapshot that would not read; restored the newest one that would"
    );
    publish_workspace_event(
        state,
        "workspace_snapshot_unreadable",
        "A stored workspace snapshot could not be read, so this pod restored the newest one it could read. If the unreadable snapshot was the newer one, work saved after the restored snapshot is not in this workspace.",
    );
}

/// Queue one sandbox event carrying `message` under `sub_type`.
fn publish_workspace_event(state: &SandboxState, sub_type: &str, message: &str) {
    let payload = serde_json::json!({ "format": restic::FORMAT, "message": message });
    match serde_json::to_string(&payload) {
        Ok(payload_json) => {
            state.events.enqueue(sandbox_proto::PublishEventRequest {
                task_id: state.config.task_id.clone(),
                event_type: sandbox_proto::EventType::Sandbox as i32,
                sub_type: sub_type.to_string(),
                payload_json,
                event_id: String::new(),
                sequence: 0,
                run_id: String::new(),
            });
        }
        Err(e) => warn!(error = %e, sub_type, "Failed to serialize a workspace sync event"),
    }
}

/// Delete every git lock the tree this pod starts on carries.
///
/// A lock arrives from a snapshot older than the exclusion or from a volume
/// the dead pod kept, so call after the restore and before the clone, off any
/// sync switch: `pvc` never restores yet keeps its volume across a kill, and
/// gating this would skip the one tier nothing else repairs.
pub async fn sweep_git_locks(workspace_dir: PathBuf) {
    let removed = tokio::task::spawn_blocking(move || {
        let locks = file_set::collect_git_locks(&workspace_dir);
        locks
            .iter()
            .filter(|lock| std::fs::remove_file(lock).is_ok())
            .count()
    })
    .await
    .unwrap_or(0);
    if removed > 0 {
        info!(removed, "Removed git locks left by a pod that is gone");
    }
}

/// Restore a full snapshot from S3 before the clone; see [`RestoreDecision`].
///
/// Runs after credentials and git auth, before the clone. Every failure path
/// returns [`RestoreDecision::Nothing`], so the caller clones and
/// [`download_on_start`] overlays instead.
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
    let client = match http_client(state.config.http.workspace_transfer) {
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
            // The tar carries no snapshot time, so the note says "unknown"
            // rather than dating the tree from this pod's own clock.
            RestoreDecision::RestoredSnapshot {
                taken_at: None,
                final_flush: false,
            }
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
/// Reached only when [`restore_on_start`] declined. Runs after the clone and
/// before Ready, extracting the tar *over* it — repo-relative onto `repo_dir`,
/// existing files overwritten, new ones added. Non-fatal.
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
    let client = match http_client(state.config.http.workspace_transfer) {
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
    // This overlay lands after init already swept, and an older tar is exactly
    // where a stored lock comes from.
    sweep_git_locks(repo_dir).await;
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

/// Store everything under `workspace_dir` in each enabled format, unless
/// unchanged since the last upload.
///
/// No-op when there's nothing to sync. Best-effort: failures are logged, never
/// propagated.
pub async fn upload(state: &SandboxState) -> SyncOutcome {
    state
        .workspace_sync
        .upload_if_changed(
            &state.config,
            state.backend_client.as_ref(),
            Prune::WhenDue,
            Capture::Periodic,
            &Phases::start(),
        )
        .await
}

/// Start the turn-boundary upload and return without waiting for the transfer.
///
/// Never awaited by the caller (the agent coordinator's main loop): a slow
/// transfer delays the *next* upload ([`TurnUpload`]), not the agent;
/// [`flush_on_shutdown`] shares its locks and races it safely.
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
/// The normal durability point between turns, and on the PVC tier the only
/// periodic one since the loop is off there. Reached through
/// [`spawn_after_turn`], off the main loop.
async fn upload_after_turn(
    config: &Config,
    backend_client: &dyn BackendClient,
    tracker: &WorkspaceSyncTracker,
) {
    if config.workspace_sync_enabled {
        tracker
            .upload_if_changed(
                config,
                backend_client,
                Prune::WhenDue,
                Capture::Periodic,
                &Phases::start(),
            )
            .await;
    }
}

/// Answer the backend's pre-stop `SyncWorkspace` before the pod is destroyed.
///
/// Prunes unconditionally rather than on the countdown, which the upload skips
/// here: this is the one moment with no next turn for the index rewrite to slow
/// down, and claiming the countdown too would rewrite it twice in a row.
pub async fn sync_on_request(state: &SandboxState) -> SyncOutcome {
    if !state.config.workspace_sync_enabled {
        debug!(task_id = %state.config.task_id, "SyncWorkspace requested but workspace sync is off");
        return SyncOutcome::default();
    }
    let outcome = state
        .workspace_sync
        .upload_if_changed(
            &state.config,
            state.backend_client.as_ref(),
            Prune::Never,
            // Not `ShutdownFlush`: the backend asks for this while the agent is
            // still free to write, so the tree it stores is not a final one.
            Capture::Periodic,
            &Phases::start(),
        )
        .await;
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
/// Unlike every other path here, not best-effort: the caller is a short-lived
/// pod with no task token or gRPC channel, only the presigned URL, and it
/// deletes the volume on success, so a silent failure here loses work.
pub async fn rescue_upload(
    workspace_dir: &str,
    put_url: &str,
    transfer: HttpTimeouts,
) -> Result<u64, String> {
    let tar = build_tar(PathBuf::from(workspace_dir))
        .await
        .ok_or_else(|| format!("no workspace archive could be built from {workspace_dir}"))?;
    let bytes = tar.len() as u64;
    http_put(&http_client(transfer)?, put_url, tar).await?;
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
    let error = match http_client(config.http.workspace_transfer) {
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

/// Periodically upload the workspace when the tracked file set has changed.
///
/// The between-turns durability backstop, running only when the volume is not
/// durable ([`is_periodic_enabled`]); each tick delegates to
/// [`WorkspaceSyncTracker`], which fingerprints and uploads only when it moved.
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
/// Called after the agent has stopped and before the lifecycle token is
/// cancelled; a timeout does not block the drain.
pub async fn flush_on_shutdown(state: &SandboxState) {
    flush_workspace(
        &state.config,
        state.backend_client.as_ref(),
        &state.workspace_sync,
        workspace_writers_are_quiet(state).await,
    )
    .await;
}

/// Whether no writer this pod can observe was live when the flush began.
///
/// Two halves: what this pod registered, then what it did not. The second reads
/// the PID namespace, so it answers `false` anywhere this process shares one
/// with processes it did not start.
async fn workspace_writers_are_quiet(state: &SandboxState) -> bool {
    if !registered_writers_are_quiet(state).await {
        return false;
    }
    // The writers nothing above registers: an agent tool's own child, a process
    // orphaned by a command that exited, one detached with `setsid`.
    match crate::pod_processes::foreign_live_processes() {
        Some(live) if live.is_empty() => true,
        Some(live) => {
            info!(
                ?live,
                "Final-tree tag withheld: the pod still has live writers"
            );
            false
        }
        None => false,
    }
}

/// Whether every writer this pod has a registry for has finished.
///
/// Each check names a writer that outlives `agent.stop()`.
async fn registered_writers_are_quiet(state: &SandboxState) -> bool {
    // A writer admitted before the close but not finished by the end of the
    // drain is one no registry below lists yet.
    if !state.writer_admission.is_drained() {
        return false;
    }
    if !matches!(*state.init_phase.read().await, InitPhase::Ready) {
        return false;
    }
    // Registered but not yet spawned: a session or command whose process the
    // scan below cannot see yet still writes once it starts.
    if state.terminals.has_live_sessions().await {
        return false;
    }
    for context in state.task_contexts.read().await.values() {
        if context.commands.busy().await {
            return false;
        }
        // A cancelled agent whose harness has not wound down yet is still
        // writing, and `AgentState::stop` only asks it to stop.
        if !matches!(
            context.agent.status().await,
            AgentStatus::Idle | AgentStatus::Stopped
        ) {
            return false;
        }
    }
    true
}

/// The body of [`flush_on_shutdown`], over what it actually needs.
async fn flush_workspace(
    config: &Config,
    backend_client: &dyn BackendClient,
    tracker: &WorkspaceSyncTracker,
    writers_quiet: bool,
) {
    let phases = Phases::start();
    if tokio::time::timeout(
        SHUTDOWN_FLUSH_TIMEOUT,
        tracker.upload_if_changed(
            config,
            backend_client,
            Prune::Never,
            Capture::ShutdownFlush { writers_quiet },
            &phases,
        ),
    )
    .await
    .is_ok()
    {
        info!(phases = %phases.summary(), "Shutdown: flushed workspace to S3");
        return;
    }

    // The cancelled upload never reached its own `report_workspace_sync`, so
    // this is all that tells the task its last turn's work is not stored.
    let error = format!(
        "the shutdown flush exceeded its budget in {} ({}); the workspace was not stored",
        phases.current().unwrap_or("start"),
        phases.summary()
    );
    warn!(%error, "Shutdown: workspace flush timed out, continuing");
    let _ = tokio::time::timeout(
        TIMEOUT_REPORT_TIMEOUT,
        backend_client.report_workspace_sync(
            &config.task_id,
            "upload",
            FORMAT_UNKNOWN,
            0,
            phases.elapsed().as_millis() as u64,
            Some(&error),
        ),
    )
    .await;
}

/// True when a gRPC error means the workspace-sync feature is simply
/// unavailable (backend S3 not configured) rather than a real transient
/// failure.
pub(super) fn is_feature_disabled(status: &tonic::Status) -> bool {
    matches!(
        status.code(),
        tonic::Code::FailedPrecondition | tonic::Code::Unimplemented
    )
}

/// The workspace-relative prefix of the primary repo (e.g. `"soap-tm"`), or
/// `""` when the primary repo *is* the workspace dir (the no-clone-URL
/// fallback).
pub(super) fn primary_prefix(workspace_dir: &str, repo_dir: &str) -> String {
    match Path::new(repo_dir).strip_prefix(Path::new(workspace_dir)) {
        Ok(rel) => rel.to_string_lossy().trim_end_matches('/').to_string(),
        Err(_) => String::new(),
    }
}

/// Build a reqwest client for transfers bounded by `timeouts`.
///
/// One built per transfer — cheap at the sync cadence — keeps no long-lived
/// connection open against the presigned host.
#[expect(
    clippy::disallowed_methods,
    reason = "sets the timeouts `Config::http` holds"
)]
pub(super) fn http_client(timeouts: HttpTimeouts) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(timeouts.connect)
        .timeout(timeouts.total)
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))
}

/// `PUT` bytes to a presigned S3 URL.
///
/// `Content-Length` is set by hand: for an empty body reqwest sends neither it
/// nor `Transfer-Encoding`, and S3 answers an unframed PUT with `411 Length
/// Required`.
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

/// `GET` bytes from a presigned S3 URL. `Ok(None)` on 404 — nothing stored
/// under that key yet.
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
/// relative to `root`. Returns whether anything was added.
///
/// A git repo root goes to [`append_repo`] instead; symlinks and non-regular
/// files are skipped so the walk can't loop or escape `root`.
fn append_tree<W: std::io::Write>(builder: &mut tar::Builder<W>, root: &Path, dir: &Path) -> bool {
    // A git repo root: enumerate via git (honors gitignore) and stop descending.
    if file_set::is_repo_root(dir) {
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
/// File set: `git ls-files -c -o --exclude-standard` (tracked + untracked, non-
/// ignored), plus `.git` wholesale, so each repo is complete on restore.
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
    let git_prefix = PathBuf::from(join_prefix(prefix, ".git"));
    let stored_git = if git_dir.is_dir() {
        append_git_dir(builder, &git_prefix, &git_dir)
    } else {
        git_dir.is_file() && builder.append_path_with_name(&git_dir, &git_prefix).is_ok()
    };
    any |= stored_git;

    any
}

/// Append everything under a repo's `.git` to `builder`, keyed at
/// `<name_prefix>/…`. Returns whether anything was added.
///
/// `append_dir_all` cannot express the one exclusion [`file_set::is_git_lock`]
/// describes; directory entries are kept so an empty one still restores. Names
/// stay `Path`s because a git ref name is bytes, not UTF-8.
fn append_git_dir<W: std::io::Write>(
    builder: &mut tar::Builder<W>,
    name_prefix: &Path,
    dir: &Path,
) -> bool {
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
        let Some(file_name) = path.file_name() else {
            continue;
        };
        let name = name_prefix.join(file_name);
        if meta.is_dir() {
            if builder.append_path_with_name(&path, &name).is_ok() {
                any = true;
            }
            if append_git_dir(builder, &name, &path) {
                any = true;
            }
        } else if meta.is_file()
            && !file_set::is_git_lock(&path)
            && builder.append_path_with_name(&path, &name).is_ok()
        {
            any = true;
        }
        // symlinks / special files are intentionally skipped
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

/// Build an in-memory **gzipped** tar of all data under `workspace_dir`, or
/// `None` when there's nothing to archive.
///
/// Tar entries stream into the encoder, so only the compressed result is held
/// in memory, inside a sandbox where memory pressure is what makes this matter.
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
/// Archives are written gzipped; one without the gzip magic is passed through
/// untouched so it still restores.
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
/// Walks the exact file set [`build_tar`] would archive and hashes `(path, len,
/// mtime)` for each; `.gitignore`d artifacts are excluded, so build churn
/// doesn't move the fingerprint. `None` when there's nothing to archive.
async fn workspace_fingerprint(workspace_dir: PathBuf) -> Option<u64> {
    tokio::task::spawn_blocking(move || fingerprint_now(&workspace_dir))
        .await
        .ok()
        .flatten()
}

/// [`workspace_fingerprint`] for a caller already on a blocking thread, which
/// is how `restic::snapshot` reads the same tree either side of one capture.
pub(super) fn fingerprint_now(workspace_dir: &Path) -> Option<u64> {
    if !workspace_dir.is_dir() {
        return None;
    }
    let mut entries: Vec<(String, u64, u128)> = Vec::new();
    fingerprint_tree(&mut entries, workspace_dir, workspace_dir);
    if entries.is_empty() {
        return None;
    }
    entries.sort();
    let mut hasher = DefaultHasher::new();
    entries.hash(&mut hasher);
    Some(hasher.finish())
}

/// Metadata twin of [`append_tree`]: a git repo root is enumerated via
/// [`fingerprint_repo`] and not descended into.
///
/// Symlinks/special files are skipped, matching the tar walk.
fn fingerprint_tree(entries: &mut Vec<(String, u64, u128)>, root: &Path, dir: &Path) {
    if file_set::is_repo_root(dir) {
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

/// Metadata twin of [`append_repo`]: the working-tree file set from `git
/// ls-files -c -o --exclude-standard` plus the whole `.git` directory.
///
/// Contributes `(len, mtime)` for each file and symlink instead of its bytes.
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
    let git = repo_dir.join(".git");
    match std::fs::symlink_metadata(&git) {
        Ok(meta) if meta.is_file() => {
            entries.push((join_prefix(prefix, ".git"), meta.len(), mtime_nanos(&meta)));
        }
        _ => fingerprint_dir(entries, &join_prefix(prefix, ".git"), &git),
    }
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
        } else if meta.is_file() && !file_set::is_git_lock(&path) {
            entries.push((name, meta.len(), mtime_nanos(&meta)));
        }
    }
}

/// A file's mtime as nanoseconds since the Unix epoch, or `0` when unavailable
/// — a stable-enough change signal for the periodic fingerprint.
fn mtime_nanos(meta: &std::fs::Metadata) -> u128 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos())
}

/// Extract a repo-set archive, picking the extraction root by its shape.
///
/// `Ok(true)`: the primary repo (with `.git`) was restored, so the caller skips
/// the clone; `Ok(false)`: working-tree-only, caller clones and overlays. The
/// `tar` crate rejects path-traversal (`..`) entries by default.
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
/// failed restore leaves an empty directory for the fallback clone. Failures
/// are logged, never propagated.
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
