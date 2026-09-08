//! The workspace stored as one S3 object per file, with a manifest.
//!
//! [`workspace_sync`](super::sync) writes the same file set as a
//! single gzipped tar. This module writes each file to
//! `<prefix>/<task_id>/files/<workspace-relative-path>` and lists them in
//! `<prefix>/<task_id>/manifest.json.gz`, so a reader holding S3 credentials can
//! open one file without the sandbox and without unpacking an archive.
//!
//! Three properties the tar does not have, and one it does:
//!
//! - **The manifest is the commit point.** Objects go up first and the manifest
//!   last, so a manifest a restore can read only ever names objects already
//!   written. A sync that dies halfway leaves the previous manifest in place.
//! - **A sync ships the delta.** [`diff`] compares the walk against the last
//!   manifest this process wrote, so an idle workspace ships nothing and a
//!   one-file edit ships one file.
//! - **Nothing is compressed**, which is what makes a stored object readable on
//!   its own — and why the same tree occupies 1.4x to 2.6x more than the tar.
//! - The tar is atomic and this is not: a restore of a mirror written by a run
//!   that died mid-sync mixes two generations of the tree. The manifest bounds
//!   that to files whose object landed before the crash.
//!
//! The sandbox still holds no AWS credentials: every transfer runs against a
//! presigned URL the backend mints per path (`GetWorkspaceObjectUrls`).

use std::collections::HashMap;
use std::future::Future;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures::stream::{self, StreamExt};
use sandbox_proto::workspace_manifest::{Entry, MANIFEST_VERSION, Manifest};
use tokio::sync::Semaphore;
// Deliberately tokio's clock, not `std`'s: a test that drives a permanently
// failing restore terminates only when the budget expires, and under
// `#[tokio::test(start_paused = true)]` only this clock advances with the
// backoff sleeps. On `std`'s the same test spends the budget in real time,
// which is how one of them hit nextest's 300s per-test ceiling.
use tokio::time::Instant;
use tracing::{debug, info, warn};

use super::file_set;
use super::sync::{
    RestoreAttempt, RestoreDecision, SyncOutcome, clear_dir_contents, http_client, http_get,
    http_put, is_feature_disabled, primary_prefix,
};
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::state::SandboxState;

/// The `format` label this module reports every transfer under.
pub(super) const FORMAT: &str = "mirror";

/// Paths per `GetWorkspaceObjectUrls` call. Must not exceed the backend's own
/// cap (`WORKSPACE_OBJECT_URL_BATCH`), which rejects a larger request.
const URL_BATCH: usize = 500;

/// Attempts an upload makes at one transfer before the sync fails.
pub(super) const UPLOAD_ATTEMPTS: u32 = 3;

/// Wall clock one restore may spend retrying, across every transfer it makes.
///
/// The pod's startup probe binds this, not the presign TTL. `restore_on_start`
/// runs inside `init::run_core_init` with `/health` still 503, and the probe
/// kills the container after `K8S_POD_STARTUP_TIMEOUT_SECS / 5 + 6` failed
/// checks at 5s spacing — 150s at the default. A budget past what is left is
/// SIGKILLed mid-backoff, so neither `report_failed_restore` nor
/// `hold_for_unread_snapshot` runs and the restart repeats the same wait. This
/// shares that window with `credentials::fetch::RETRY_BUDGET` (90s) ahead of it
/// and `init::git::CLONE_RETRY_BUDGET` (45s) after it; the container restart is
/// the outer retry.
const RESTORE_RETRY_BUDGET: Duration = Duration::from_secs(45);

/// First wait between attempts, doubled on each one up to [`BACKOFF_CAP`].
const BACKOFF_START: Duration = Duration::from_millis(250);
const BACKOFF_CAP: Duration = Duration::from_secs(5);

/// Transfers in flight. Each object is a separate request, so the wall clock is
/// round trips divided by this; 16 puts a few thousand objects inside a couple
/// of seconds without opening a connection per file.
const TRANSFER_CONCURRENCY: usize = 16;

/// Megabytes of file body held in memory across all in-flight transfers.
///
/// Concurrency alone does not bound this: 16 git pack files are gigabytes. Each
/// transfer takes permits for its own size, so one large object runs nearly
/// alone and small ones still run 16 at a time.
const INFLIGHT_BUDGET_MB: usize = 256;

/// Per-object HTTP timeout. One file, not the whole workspace — a stuck object
/// must not hold the sync open the way the tar's five-minute budget could.
const OBJECT_TIMEOUT: Duration = Duration::from_mins(1);

/// Largest object a single `PUT` can carry. S3 rejects more; a file this size in
/// a workspace is skipped rather than failing every sync from here on.
const MAX_OBJECT_BYTES: u64 = 5 * 1024 * 1024 * 1024;

/// How hard a transfer retries a failure that may be transient.
///
/// The two directions have opposite stakes. A failed upload retries on the next
/// sync tick, so it costs one interval and gives up after [`UPLOAD_ATTEMPTS`].
/// A failed restore clears the workspace, clones over the snapshot and holds
/// every upload for the pod's life, so it spends [`RESTORE_RETRY_BUDGET`] of
/// wall clock before accepting that.
#[derive(Clone, Copy)]
pub(super) enum Retry {
    Upload,
    /// Retry until this instant, whatever number of attempts that allows. One
    /// restore shares one deadline across every transfer it makes, so a restore
    /// that is merely slow cannot extend its own budget.
    Restore(Instant),
}

impl Retry {
    /// A budget for one whole restore, spent from now.
    pub(super) fn restore() -> Self {
        Self::Restore(Instant::now() + RESTORE_RETRY_BUDGET)
    }

    /// Whether an operation that has failed `attempts` times may try again.
    fn again(self, attempts: u32) -> bool {
        match self {
            Self::Upload => attempts < UPLOAD_ATTEMPTS,
            Self::Restore(deadline) => Instant::now() < deadline,
        }
    }
}

/// One attempt at a transfer. `Err(_, false)` is an answer rather than a blip,
/// and stops the retry loop at once.
type Attempt<T> = Result<T, (String, bool)>;

/// Run `op` until it succeeds, answers definitively, or `retry` is spent.
async fn with_retry<T, F, Fut>(retry: Retry, what: &str, mut op: F) -> Result<T, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Attempt<T>>,
{
    let mut attempts = 0;
    let mut delay = BACKOFF_START;
    loop {
        match op().await {
            Ok(value) => return Ok(value),
            Err((error, retryable)) => {
                attempts += 1;
                if !retryable || !retry.again(attempts) {
                    return Err(error);
                }
                warn!(%error, attempts, "{what} failed; retrying");
                tokio::time::sleep(delay + jitter(delay)).await;
                delay = (delay * 2).min(BACKOFF_CAP);
            }
        }
    }
}

/// Up to a quarter of `delay`, taken from the clock because this crate carries
/// no RNG. A node reclaim restarts every pod on the node at once, and a fixed
/// backoff would have all of them retry against the backend in step.
fn jitter(delay: Duration) -> Duration {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| u64::from(since.subsec_nanos()));
    Duration::from_nanos((delay.as_nanos() as u64 / 4).saturating_mul(nanos) / 1_000_000_000)
}

/// What one sync has to move.
#[derive(Debug, Default)]
struct Delta {
    /// Entries whose object must be written. Symlinks are carried here too, and
    /// the transfer step skips them — they exist only in the manifest.
    changed: Vec<Entry>,
    /// Paths whose object must be deleted.
    removed: Vec<String>,
}

impl Delta {
    fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty()
    }
}

/// What the mirror must ship to take `current` from `baseline`.
///
/// A `None` baseline is a workspace this process has not shipped, so everything
/// is new. That is the first sync after a boot, and it is also what a failed
/// sync leaves behind, so a retry re-checks the whole tree rather than trusting
/// a half-written one.
fn diff(baseline: Option<&Manifest>, current: &Manifest) -> Delta {
    let previous: HashMap<&str, &Entry> = baseline
        .map(|m| m.entries.iter().map(|e| (e.path.as_str(), e)).collect())
        .unwrap_or_default();

    let changed = current
        .entries
        .iter()
        .filter(|entry| {
            previous
                .get(entry.path.as_str())
                .is_none_or(|old| !old.matches(entry))
        })
        .cloned()
        .collect();

    let current_paths: std::collections::HashSet<&str> =
        current.entries.iter().map(|e| e.path.as_str()).collect();
    let removed = previous
        .values()
        .filter(|entry| !entry.is_symlink() && !current_paths.contains(entry.path.as_str()))
        .map(|entry| entry.path.clone())
        .collect();

    Delta { changed, removed }
}

/// Walk the workspace into a manifest, or `None` when it holds nothing.
///
/// The file set is [`file_set::collect`]'s. Runs on a blocking thread (`git`
/// subprocess plus `std::fs`).
pub(super) async fn build_manifest(workspace_dir: PathBuf) -> Option<Manifest> {
    tokio::task::spawn_blocking(move || {
        let mut entries = Vec::new();
        for file in file_set::collect(&workspace_dir) {
            push_entry(&mut entries, file.rel, &file.full, &file.meta);
        }
        if entries.is_empty() {
            return None;
        }
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Some(Manifest::new(entries))
    })
    .await
    .ok()
    .flatten()
}

/// Record one path, resolving a symlink to its target rather than reading
/// through it. A path the backend would refuse as a key is dropped here, so a
/// single unmirrorable file cannot fail every sync.
fn push_entry(entries: &mut Vec<Entry>, path: String, full: &Path, meta: &std::fs::Metadata) {
    if !sandbox_proto::is_safe_workspace_path(&path) {
        warn!(path = %path, "Skipping a workspace path the mirror cannot key");
        return;
    }
    let symlink = if meta.is_symlink() {
        match std::fs::read_link(full) {
            Ok(target) => Some(target.to_string_lossy().to_string()),
            Err(_) => return,
        }
    } else {
        None
    };
    if symlink.is_none() && meta.len() > MAX_OBJECT_BYTES {
        warn!(path = %path, bytes = meta.len(), "Skipping a workspace file too large for one S3 object");
        return;
    }
    let is_link = symlink.is_some();
    entries.push(Entry {
        path,
        size: if is_link { 0 } else { meta.len() },
        // A symlink's own mtime cannot be reproduced by the restore, so
        // recording it would make every post-restore sync ship the manifest.
        mtime_ns: if is_link { 0 } else { mtime_nanos(meta) },
        mode: meta.permissions().mode() & 0o7777,
        symlink,
    });
}

fn mtime_nanos(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}

/// Mirror the workspace, or report that S3 holds no mirror generation for it.
///
/// `Ok(outcome)` means the mirror is the format in use and `outcome` says what
/// moved. `Err(())` means this tree reached no mirror — the backend minted no
/// manifest URL (it predates the mirror), or the transfer failed — and the
/// caller must write a tar so the task keeps a durable copy. A failed transfer
/// leaves `baseline` alone, so the next attempt re-ships the same delta.
pub(super) async fn upload(
    config: &Config,
    backend_client: &dyn BackendClient,
    baseline: &mut Option<Manifest>,
) -> Result<SyncOutcome, ()> {
    let task_id = &config.task_id;

    let Some(manifest) = build_manifest(PathBuf::from(&config.workspace_dir)).await else {
        debug!(task_id = %task_id, "Workspace is empty; nothing to mirror");
        return Ok(SyncOutcome::default());
    };

    // Diff before touching the backend: an unchanged workspace ships nothing and
    // mints no URL, the same gate the tar path applies with its fingerprint.
    let delta = diff(baseline.as_ref(), &manifest);
    if delta.is_empty() {
        debug!(task_id = %task_id, "Workspace unchanged; skipping mirror upload");
        return Ok(SyncOutcome::default());
    }

    let urls = match backend_client.get_workspace_sync_urls(task_id).await {
        Ok(urls) if !urls.manifest_put_url.is_empty() => urls,
        Ok(_) => {
            debug!(task_id = %task_id, "Backend mints no manifest URL; falling back to the tar");
            return Err(());
        }
        Err(e) if is_feature_disabled(&e) => {
            debug!(task_id = %task_id, "Workspace sync unavailable (backend S3 not configured)");
            return Ok(SyncOutcome::default());
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Failed to mint workspace sync URLs (non-fatal)");
            return Ok(SyncOutcome::default());
        }
    };

    let started = Instant::now();
    let workspace_dir = PathBuf::from(&config.workspace_dir);
    let outcome = mirror_delta(
        task_id,
        backend_client,
        &workspace_dir,
        &delta,
        &manifest,
        &urls.manifest_put_url,
    )
    .await;

    let (error, bytes) = match outcome {
        Ok(bytes) => {
            info!(
                task_id = %task_id,
                files = delta.changed.len(),
                removed = delta.removed.len(),
                bytes = bytes,
                "Mirrored workspace changes to S3"
            );
            *baseline = Some(manifest);
            (None, bytes)
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Workspace mirror upload failed (non-fatal)");
            (Some(e), 0)
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
        return Err(());
    }
    Ok(SyncOutcome {
        synced: true,
        bytes,
    })
}

/// Write the changed objects, delete the removed ones, then write the manifest.
///
/// # Call the manifest last
///
/// The manifest is what a restore reads, so writing it before its objects would
/// name files that are not there. Every failure short-circuits ahead of it,
/// leaving the previous manifest as the readable generation.
async fn mirror_delta(
    task_id: &str,
    backend_client: &dyn BackendClient,
    workspace_dir: &Path,
    delta: &Delta,
    manifest: &Manifest,
    manifest_put_url: &str,
) -> Result<u64, String> {
    let client = http_client(OBJECT_TIMEOUT)?;

    let files: Vec<&Entry> = delta.changed.iter().filter(|e| !e.is_symlink()).collect();
    let paths: Vec<String> = files.iter().map(|e| e.path.clone()).collect();
    let sizes: HashMap<&str, u64> = files.iter().map(|e| (e.path.as_str(), e.size)).collect();
    let mut uploaded = 0u64;

    // Mint each batch immediately before its transfers rather than all of them
    // up front, so a URL is never older than the batch that uses it.
    for chunk in paths.chunks(URL_BATCH) {
        let urls = mint_urls(backend_client, task_id, chunk, "put", false, Retry::Upload).await?;
        let budget = Arc::new(Semaphore::new(INFLIGHT_BUDGET_MB));
        // Own each (path, url): a closure taking a borrowed path fails the
        // higher-ranked-lifetime bound `buffer_unordered` needs on its futures.
        let transfers = chunk.iter().cloned().zip(urls).map(|(path, url)| {
            let client = client.clone();
            let budget = Arc::clone(&budget);
            let full = workspace_dir.join(&path);
            let permits = permits_for(sizes.get(path.as_str()).copied().unwrap_or(0));
            async move {
                let _permit = budget
                    .acquire_many(permits)
                    .await
                    .map_err(|e| format!("mirror budget closed: {e}"))?;
                put_file(&client, &url, &full, Retry::Upload).await
            }
        });
        let results: Vec<Result<u64, String>> = stream::iter(transfers)
            .buffer_unordered(TRANSFER_CONCURRENCY)
            .collect()
            .await;
        for result in results {
            uploaded += result?;
        }
    }

    for chunk in delta.removed.chunks(URL_BATCH) {
        let urls = mint_urls(
            backend_client,
            task_id,
            chunk,
            "delete",
            false,
            Retry::Upload,
        )
        .await?;
        let deletes = urls.into_iter().map(|url| {
            let client = client.clone();
            async move { delete_object(&client, &url).await }
        });
        let results: Vec<Result<(), String>> = stream::iter(deletes)
            .buffer_unordered(TRANSFER_CONCURRENCY)
            .collect()
            .await;
        for result in results {
            result?;
        }
    }

    let body = manifest
        .to_gz_json()
        .map_err(|e| format!("encoding the manifest failed: {e}"))?;
    let manifest_bytes = body.len() as u64;
    http_put(&client, manifest_put_url, body).await?;

    Ok(uploaded + manifest_bytes)
}

/// The stored manifest: `Ok(None)` when this task has no mirror, `Err` when one
/// may exist and this sandbox could not read it.
///
/// The caller descends to an older format on `Ok(None)` only, so a failure that
/// leaves the answer unknown — a timeout minting the URL, a download error, a
/// manifest written by a newer build — must be an `Err` rather than a `None`.
pub(super) async fn fetch_stored_manifest(
    task_id: &str,
    backend_client: &dyn BackendClient,
    client: &reqwest::Client,
    retry: Retry,
) -> Result<Option<Manifest>, String> {
    let minted = with_retry(retry, "Minting workspace sync URLs", || async {
        match backend_client.get_workspace_sync_urls(task_id).await {
            Ok(urls) => Ok(Some(urls)),
            // Not an outage: the backend is telling us this deployment stores
            // no workspace at all, so there is nothing for any format to read.
            Err(status) if is_feature_disabled(&status) => Ok(None),
            Err(status) => Err((format!("minting sync URLs failed: {status}"), true)),
        }
    })
    .await?;
    let Some(urls) = minted else {
        return Ok(None);
    };
    if urls.manifest_get_url.is_empty() {
        debug!(task_id = %task_id, "Backend minted no manifest URL");
        return Ok(None);
    }

    let fetched = with_retry(retry, "Downloading the stored manifest", || async {
        http_get(client, &urls.manifest_get_url)
            .await
            .map_err(|e| (format!("manifest download failed: {e}"), e.retryable))
    })
    .await?;

    let Some(bytes) = fetched else {
        debug!(task_id = %task_id, "No stored workspace mirror");
        return Ok(None);
    };
    match Manifest::from_gz_json(&bytes) {
        Ok(manifest) if manifest.version == MANIFEST_VERSION => Ok(Some(manifest)),
        Ok(manifest) => Err(format!(
            "stored manifest is version {}, which this build cannot read",
            manifest.version
        )),
        Err(e) => Err(format!("stored manifest could not be parsed: {e}")),
    }
}

/// The delta base an upload starts a pod with: the manifest S3 holds, never a
/// walk of the tree on disk.
///
/// A clone is not a mirror. A workspace that was cloned, or restored from a tar,
/// has no object in S3 behind any of its files, so adopting it would make the
/// first upload ship only what the agent changed afterwards while writing a
/// manifest naming thousands of objects nobody stored — and the next restore
/// fails on the first of them, clears the workspace and clones over the work.
/// `None` means the first upload ships the whole tree, which is what makes the
/// manifest true.
pub(super) async fn stored_baseline(
    task_id: &str,
    backend_client: &dyn BackendClient,
) -> Option<Manifest> {
    let client = http_client(OBJECT_TIMEOUT).ok()?;
    // A manifest this sandbox could not read is no delta base either, so both
    // failure shapes collapse to `None` here.
    fetch_stored_manifest(task_id, backend_client, &client, Retry::Upload)
        .await
        .ok()
        .flatten()
}

/// Restore the workspace from the mirror.
///
/// [`RestoreAttempt::Absent`] means no mirror is stored for this task (or the
/// backend predates it) and the caller may fall back to the tar;
/// [`RestoreAttempt::Failed`] means one may exist unread, and the caller must
/// not. The shape rules match `workspace_sync::restore_on_start`'s: the mirror
/// is a complete set of repos when it carries the primary repo's `.git`, and
/// paths are workspace-relative unless the manifest came from the single-repo
/// shape.
///
/// **Failure policy:** a failed transfer clears the workspace and returns
/// [`RestoreAttempt::Failed`], so the caller clones instead of running on half a
/// tree — and does not descend to the tar.
pub(super) async fn restore_on_start(state: &SandboxState) -> RestoreAttempt {
    let task_id = &state.config.task_id;
    let workspace_dir = PathBuf::from(&state.config.workspace_dir);
    let repo_dir = PathBuf::from(&state.config.repo_dir);
    let prefix = primary_prefix(&state.config.workspace_dir, &state.config.repo_dir);

    let started = Instant::now();
    let Ok(client) = http_client(OBJECT_TIMEOUT) else {
        warn!(task_id = %task_id, "Mirror restore client build failed; cannot tell whether a mirror exists");
        return RestoreAttempt::Failed;
    };
    // One budget for the whole restore, so a batch that keeps failing cannot
    // spend more than the restore as a whole is allowed.
    let retry = Retry::restore();
    let manifest =
        match fetch_stored_manifest(task_id, state.backend_client.as_ref(), &client, retry).await {
            Ok(Some(manifest)) => manifest,
            Ok(None) => return RestoreAttempt::Absent,
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Stored manifest could not be read");
                report_failed_restore(state, started, &e).await;
                return RestoreAttempt::Failed;
            }
        };

    // Same three shapes the tar restore classifies, decided off the manifest
    // rather than by scanning an archive.
    let target = if manifest.carries_git(&prefix) {
        workspace_dir.clone()
    } else if manifest.carries_git("") {
        // A single-repo manifest written repo-relative belongs on `repo_dir`.
        repo_dir.clone()
    } else {
        warn!(task_id = %task_id, "Stored manifest carries no primary .git; falling back to the tar");
        state.workspace_sync.mark_mirror_unusable();
        return RestoreAttempt::Absent;
    };

    let bytes = match download_manifest_files(state, &client, &manifest, &target, retry).await {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Workspace mirror restore failed; clearing partial state");
            state.workspace_sync.mark_mirror_unusable();
            clear_dir_contents(&workspace_dir).await;
            report_failed_restore(state, started, &e).await;
            return RestoreAttempt::Failed;
        }
    };

    info!(
        task_id = %task_id,
        files = manifest.entries.len(),
        bytes = bytes,
        dir = %target.display(),
        "Restored repos from the S3 workspace mirror (skipping clone)"
    );
    state
        .backend_client
        .report_workspace_sync(
            task_id,
            "download",
            FORMAT,
            bytes,
            started.elapsed().as_millis() as u64,
            None,
        )
        .await;
    RestoreAttempt::Restored(RestoreDecision::RestoredSnapshot)
}

/// Report a restore this pod could not complete.
///
/// The backend turns this into a `sandbox`/`workspace_sync_failed` event, which
/// is the only durable trace of it: the pod clones over the snapshot and its
/// container log dies with it, so the reason is otherwise unreadable by the time
/// anyone asks why the tree came back empty.
async fn report_failed_restore(state: &SandboxState, started: Instant, error: &str) {
    state
        .backend_client
        .report_workspace_sync(
            &state.config.task_id,
            "download",
            FORMAT,
            0,
            started.elapsed().as_millis() as u64,
            Some(error),
        )
        .await;
}

/// Fetch every file the manifest names into `target`, recreating symlinks from
/// the manifest rather than fetching them.
async fn download_manifest_files(
    state: &SandboxState,
    client: &reqwest::Client,
    manifest: &Manifest,
    target: &Path,
    retry: Retry,
) -> Result<u64, String> {
    for entry in manifest.entries.iter().filter(|e| e.is_symlink()) {
        write_symlink(target, entry).await?;
    }

    let files: Vec<&Entry> = manifest
        .entries
        .iter()
        .filter(|e| !e.is_symlink())
        .collect();
    let paths: Vec<String> = files.iter().map(|e| e.path.clone()).collect();
    let by_path: HashMap<&str, &Entry> = files.iter().map(|e| (e.path.as_str(), *e)).collect();
    let mut downloaded = 0u64;

    for chunk in paths.chunks(URL_BATCH) {
        let urls = mint_urls(
            state.backend_client.as_ref(),
            &state.config.task_id,
            chunk,
            "get",
            true,
            retry,
        )
        .await?;
        let budget = Arc::new(Semaphore::new(INFLIGHT_BUDGET_MB));
        let transfers = chunk.iter().cloned().zip(urls).map(|(path, url)| {
            let client = client.clone();
            let budget = Arc::clone(&budget);
            let full = target.join(&path);
            let entry = by_path.get(path.as_str()).copied().cloned();
            let permits = permits_for(entry.as_ref().map_or(0, |e| e.size));
            async move {
                let _permit = budget
                    .acquire_many(permits)
                    .await
                    .map_err(|e| format!("mirror budget closed: {e}"))?;
                get_file(&client, &url, &full, entry.as_ref(), retry).await
            }
        });
        let results: Vec<Result<u64, String>> = stream::iter(transfers)
            .buffer_unordered(TRANSFER_CONCURRENCY)
            .collect()
            .await;
        for result in results {
            downloaded += result?;
        }
    }

    Ok(downloaded)
}

/// Mint one batch of object URLs, retrying a transient backend failure.
///
/// `FailedPrecondition` and `Unimplemented` are answers rather than blips, so
/// they return on the first attempt.
pub(super) async fn mint_urls(
    backend_client: &dyn BackendClient,
    task_id: &str,
    chunk: &[String],
    operation: &str,
    restore_source: bool,
    retry: Retry,
) -> Result<Vec<String>, String> {
    with_retry(retry, "Minting workspace object URLs", || async {
        backend_client
            .get_workspace_object_urls(task_id, chunk, operation, restore_source)
            .await
            .map_err(|status| {
                (
                    format!("minting {operation} URLs failed: {status}"),
                    !is_feature_disabled(&status),
                )
            })
    })
    .await
}

/// In-flight permits one file claims, one per megabyte and never more than the
/// whole budget, so a single object larger than the budget still runs.
fn permits_for(size: u64) -> u32 {
    let mb = size.div_ceil(1024 * 1024).max(1);
    u32::try_from(mb.min(INFLIGHT_BUDGET_MB as u64)).unwrap_or(1)
}

/// `PUT` one file's bytes, returning what was sent. A file that disappeared
/// between the walk and the read is skipped: a build artifact deleted mid-sync
/// must not fail the sync and strand every later one.
async fn put_file(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    retry: Retry,
) -> Result<u64, String> {
    // A `PUT` overwrites one key, so a retry after an ambiguous failure is safe
    // whether or not the first one landed.
    let sent = with_retry(retry, "Uploading a workspace object", || async {
        // Read inside the attempt. Holding a second copy of the body for a
        // retry that may never come would double what `permits_for` reserved.
        let body = match tokio::fs::read(path).await {
            Ok(body) => body,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err((format!("reading {} failed: {e}", path.display()), false)),
        };
        let bytes = body.len() as u64;
        // Name the object: one bad file fails the whole sync, and the bare
        // transport error left no way to tell which — or how big it was.
        http_put(client, url, body)
            .await
            .map(|()| Some(bytes))
            .map_err(|e| {
                (
                    format!("{e} ({}, {bytes} bytes)", path.display()),
                    e.retryable,
                )
            })
    })
    .await?;
    Ok(sent.unwrap_or(0))
}

/// `GET` one object into `path`, creating its parent directories and restoring
/// the mode and modification time the manifest recorded.
///
/// # Restore the mtime, do not let the write set it
///
/// Two things read it. `git status` treats a file whose mtime moved as needing a
/// content re-hash, so a whole tree stamped at restore time is a slow first
/// command in every restored pod; and [`Entry::matches`] compares it, so the
/// next sync would ship every file back. The tar restores mtimes for the same
/// reason.
async fn get_file(
    client: &reqwest::Client,
    url: &str,
    path: &Path,
    entry: Option<&Entry>,
    retry: Retry,
) -> Result<u64, String> {
    let body = with_retry(retry, "Downloading a workspace object", || async {
        match http_get(client, url).await {
            // A 404 is S3 answering, not failing: the object is not there, and
            // no number of retries puts it there.
            Ok(None) => Err((
                format!(
                    "manifest names an object that is not stored: {}",
                    path.display()
                ),
                false,
            )),
            Ok(Some(body)) => Ok(body),
            Err(e) => Err((format!("{e} ({})", path.display()), e.retryable)),
        }
    })
    .await?;
    let bytes = body.len() as u64;
    let path = path.to_path_buf();
    let mode = entry.map_or(0o644, |e| e.mode);
    let mtime_ns = entry.map_or(0, |e| e.mtime_ns);
    tokio::task::spawn_blocking(move || write_restored_file(&path, &body, mode, mtime_ns))
        .await
        .map_err(|e| format!("restore write task panicked: {e}"))??;
    Ok(bytes)
}

fn write_restored_file(path: &Path, body: &[u8], mode: u32, mtime_ns: u64) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating {} failed: {e}", parent.display()))?;
    }
    let mut file = std::fs::File::create(path)
        .map_err(|e| format!("creating {} failed: {e}", path.display()))?;
    file.write_all(body)
        .map_err(|e| format!("writing {} failed: {e}", path.display()))?;
    if mode != 0 {
        file.set_permissions(std::fs::Permissions::from_mode(mode))
            .map_err(|e| format!("setting the mode on {} failed: {e}", path.display()))?;
    }
    if mtime_ns != 0 {
        file.set_modified(UNIX_EPOCH + Duration::from_nanos(mtime_ns))
            .map_err(|e| format!("setting the mtime on {} failed: {e}", path.display()))?;
    }
    Ok(())
}

/// Recreate one symlink from its manifest entry, replacing whatever is there.
async fn write_symlink(target_root: &Path, entry: &Entry) -> Result<(), String> {
    let Some(link_target) = entry.symlink.as_deref() else {
        return Ok(());
    };
    let path = target_root.join(&entry.path);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("creating {} failed: {e}", parent.display()))?;
    }
    let _ = tokio::fs::remove_file(&path).await;
    tokio::fs::symlink(link_target, &path)
        .await
        .map_err(|e| format!("linking {} failed: {e}", path.display()))
}

/// A `DELETE` of a key that is not there succeeds, which is what S3 reports and
/// what a retry of a partly-applied delta needs.
async fn delete_object(client: &reqwest::Client, url: &str) -> Result<(), String> {
    let resp = client
        .delete(url)
        .send()
        .await
        .map_err(|e| format!("DELETE request failed: {e}"))?;
    if resp.status().is_success() || resp.status() == reqwest::StatusCode::NOT_FOUND {
        Ok(())
    } else {
        Err(format!("DELETE returned HTTP {}", resp.status()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(path: &str, size: u64, mtime_ns: u64) -> Entry {
        Entry {
            path: path.to_string(),
            size,
            mtime_ns,
            mode: 0o644,
            symlink: None,
        }
    }

    /// The gate every mirror sync passes: an unchanged tree ships nothing, and
    /// each of the three ways a tree moves is picked up.
    #[test]
    fn the_delta_is_what_moved() {
        let baseline = Manifest::new(vec![
            entry("repo/a.rs", 10, 1),
            entry("repo/b.rs", 20, 2),
            entry("repo/gone.rs", 30, 3),
        ]);

        assert!(
            diff(Some(&baseline), &baseline).is_empty(),
            "an unchanged workspace ships nothing"
        );

        let current = Manifest::new(vec![
            entry("repo/a.rs", 10, 1),
            entry("repo/b.rs", 21, 9),
            entry("repo/new.rs", 5, 4),
        ]);
        let delta = diff(Some(&baseline), &current);
        assert_eq!(
            delta
                .changed
                .iter()
                .map(|e| e.path.as_str())
                .collect::<Vec<_>>(),
            vec!["repo/b.rs", "repo/new.rs"],
            "an edited and an added file both ship"
        );
        assert_eq!(delta.removed, vec!["repo/gone.rs".to_string()]);
    }

    /// No baseline is a workspace this process has not shipped, so a retry after
    /// a failure re-checks everything rather than trusting a half-written tree.
    #[test]
    fn without_a_baseline_everything_ships() {
        let current = Manifest::new(vec![entry("repo/a.rs", 10, 1), entry("repo/b.rs", 20, 2)]);
        let delta = diff(None, &current);
        assert_eq!(delta.changed.len(), 2);
        assert!(delta.removed.is_empty());
    }

    /// A mode change is a content change as far as a restore is concerned: the
    /// bytes are the same but the restored file would not be executable.
    #[test]
    fn a_mode_change_ships_the_file() {
        let baseline = Manifest::new(vec![entry("repo/run.sh", 10, 1)]);
        let mut executable = entry("repo/run.sh", 10, 1);
        executable.mode = 0o755;
        let delta = diff(Some(&baseline), &Manifest::new(vec![executable]));
        assert_eq!(delta.changed.len(), 1);
    }

    /// A symlink lives in the manifest and nowhere else, so removing one must
    /// not queue a delete for an object that was never written.
    #[test]
    fn a_removed_symlink_queues_no_delete() {
        let mut link = entry("repo/link", 0, 1);
        link.symlink = Some("target".to_string());
        let baseline = Manifest::new(vec![link, entry("repo/a.rs", 10, 1)]);
        let delta = diff(
            Some(&baseline),
            &Manifest::new(vec![entry("repo/a.rs", 10, 1)]),
        );
        assert!(delta.removed.is_empty());
    }

    #[test]
    fn a_manifest_round_trips_through_its_stored_form() {
        let manifest = Manifest::new(vec![entry("repo/a.rs", 10, 1)]);
        let stored = manifest.to_gz_json().unwrap();
        assert_eq!(Manifest::from_gz_json(&stored).unwrap(), manifest);
    }

    /// The restore's shape test: a mirror is a complete set of repos only when
    /// it carries the primary repo's `.git`.
    #[test]
    fn the_primary_git_decides_the_shape() {
        let multi = Manifest::new(vec![
            entry("repo/.git/HEAD", 10, 1),
            entry("repo/src/main.rs", 20, 2),
        ]);
        assert!(multi.carries_git("repo"));
        assert!(!multi.carries_git(""));

        let working_tree_only = Manifest::new(vec![entry("repo/src/main.rs", 20, 2)]);
        assert!(!working_tree_only.carries_git("repo"));
        assert!(!working_tree_only.carries_git(""));
    }

    #[tokio::test]
    async fn a_walk_records_the_tracked_set_and_git_but_not_ignored_files() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("workspace");
        let repo = ws.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t.co"],
            vec!["config", "user.name", "t"],
        ] {
            std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args(&args)
                .output()
                .unwrap();
        }
        std::fs::write(repo.join(".gitignore"), "ignored/\n").unwrap();
        std::fs::write(repo.join("main.rs"), "fn main() {}\n").unwrap();
        std::fs::create_dir_all(repo.join("ignored")).unwrap();
        std::fs::write(repo.join("ignored").join("build.log"), "x\n").unwrap();
        std::fs::write(ws.join("loose.txt"), "outside a repo\n").unwrap();

        let manifest = build_manifest(ws.clone())
            .await
            .expect("workspace has files");
        let paths: Vec<&str> = manifest.entries.iter().map(|e| e.path.as_str()).collect();

        assert!(paths.contains(&"repo/main.rs"));
        assert!(
            paths.contains(&"loose.txt"),
            "a file outside a repo is mirrored"
        );
        assert!(
            paths.iter().any(|p| p.starts_with("repo/.git/")),
            ".git is mirrored so the restore reproduces HEAD"
        );
        assert!(
            !paths.iter().any(|p| p.contains("ignored/")),
            "gitignored build output stays out"
        );
        assert!(
            manifest.entries.windows(2).all(|w| w[0].path < w[1].path),
            "entries are sorted so two walks of one tree produce one manifest"
        );
    }
}
