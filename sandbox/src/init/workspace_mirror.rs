//! The workspace stored as one S3 object per file, with a manifest.
//!
//! [`workspace_sync`](super::workspace_sync) writes the same file set as a
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
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use futures::stream::{self, StreamExt};
use sandbox_proto::workspace_manifest::{Entry, MANIFEST_VERSION, Manifest};
use tokio::sync::Semaphore;
use tracing::{debug, info, warn};

use super::workspace_sync::{
    RestoreDecision, SyncOutcome, clear_dir_contents, http_client, http_get, http_put, join_prefix,
    primary_prefix, rel_from,
};
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::state::SandboxState;

/// The `format` label this module reports every transfer under.
pub(super) const FORMAT: &str = "mirror";

/// Paths per `GetWorkspaceObjectUrls` call. Must not exceed the backend's own
/// cap (`WORKSPACE_OBJECT_URL_BATCH`), which rejects a larger request.
const URL_BATCH: usize = 500;

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
/// The file set is the one `workspace_sync::append_tree` archives: inside a git
/// repo the tracked and untracked-non-ignored files plus the whole `.git`, and
/// outside one every regular file. Runs on a blocking thread (`git` subprocess
/// plus `std::fs`).
pub(super) async fn build_manifest(workspace_dir: PathBuf) -> Option<Manifest> {
    tokio::task::spawn_blocking(move || {
        if !workspace_dir.is_dir() {
            return None;
        }
        let mut entries = Vec::new();
        collect_tree(&mut entries, &workspace_dir, &workspace_dir);
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

/// Metadata twin of `workspace_sync::append_tree`: a git repo root is enumerated
/// through git and not descended into, every other directory is walked.
fn collect_tree(entries: &mut Vec<Entry>, root: &Path, dir: &Path) {
    if dir.join(".git").is_dir() {
        collect_repo(entries, dir, &rel_from(root, dir));
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
            collect_tree(entries, root, &path);
        } else if meta.is_file() {
            push_entry(entries, rel_from(root, &path), &path, &meta);
        }
    }
}

/// Metadata twin of `workspace_sync::append_repo`: the working-tree file set
/// from `git ls-files -c -o --exclude-standard` plus the whole `.git`.
fn collect_repo(entries: &mut Vec<Entry>, repo_dir: &Path, prefix: &str) {
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
                let Ok(meta) = std::fs::symlink_metadata(&full) else {
                    continue;
                };
                if meta.is_file() || meta.is_symlink() {
                    push_entry(entries, join_prefix(prefix, rel), &full, &meta);
                }
            }
        }
    }
    collect_dir(
        entries,
        &join_prefix(prefix, ".git"),
        &repo_dir.join(".git"),
    );
}

/// Every regular file under `dir`, keyed at `<name_prefix>/<path-below-dir>`.
/// Used for a repo's `.git`, which has no gitignore to honor.
fn collect_dir(entries: &mut Vec<Entry>, name_prefix: &str, dir: &Path) {
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
            collect_dir(entries, &name, &path);
        } else if meta.is_file() {
            push_entry(entries, name, &path, &meta);
        }
    }
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

/// Mirror the workspace, or report that this backend cannot host a mirror.
///
/// `Ok(outcome)` means the mirror is the format in use and `outcome` says what
/// moved. `Err(())` means the backend minted no manifest URL — it predates the
/// mirror — and the caller must write a tar instead. A transfer that fails
/// returns `Ok` with `synced: false` and leaves `baseline` alone, so the next
/// attempt re-ships the same delta.
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

    let ok = error.is_none();
    Ok(SyncOutcome {
        synced: ok,
        bytes: if ok { bytes } else { 0 },
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

    for (chunk, urls) in batched_urls(backend_client, task_id, &paths, "put", false).await? {
        let budget = Arc::new(Semaphore::new(INFLIGHT_BUDGET_MB));
        // Own each (path, url): a closure taking a borrowed path fails the
        // higher-ranked-lifetime bound `buffer_unordered` needs on its futures.
        let transfers = chunk.into_iter().zip(urls).map(|(path, url)| {
            let client = client.clone();
            let budget = Arc::clone(&budget);
            let full = workspace_dir.join(&path);
            let permits = permits_for(sizes.get(path.as_str()).copied().unwrap_or(0));
            async move {
                let _permit = budget
                    .acquire_many(permits)
                    .await
                    .map_err(|e| format!("mirror budget closed: {e}"))?;
                put_file(&client, &url, &full).await
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

    for (_, urls) in batched_urls(backend_client, task_id, &delta.removed, "delete", false).await? {
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

/// The manifest S3 currently holds, or `None` when there is none this build can
/// read — no object, an unparseable one, a version this build does not know, or
/// a backend that mints no URL for it.
async fn fetch_stored_manifest(
    task_id: &str,
    backend_client: &dyn BackendClient,
    client: &reqwest::Client,
) -> Option<Manifest> {
    let urls = match backend_client.get_workspace_sync_urls(task_id).await {
        Ok(urls) if !urls.manifest_get_url.is_empty() => urls,
        _ => return None,
    };

    match http_get(client, &urls.manifest_get_url).await {
        Ok(Some(bytes)) => match Manifest::from_gz_json(&bytes) {
            Ok(manifest) if manifest.version == MANIFEST_VERSION => Some(manifest),
            Ok(manifest) => {
                warn!(task_id = %task_id, version = manifest.version, "Stored manifest is a version this build cannot read");
                None
            }
            Err(e) => {
                warn!(task_id = %task_id, error = %e, "Stored manifest could not be parsed");
                None
            }
        },
        Ok(None) => {
            debug!(task_id = %task_id, "No stored workspace mirror");
            None
        }
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Manifest download failed");
            None
        }
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
    fetch_stored_manifest(task_id, backend_client, &client).await
}

/// Restore the workspace from the mirror, or report that there is none.
///
/// `None` means no mirror is stored for this task (or the backend predates it),
/// and the caller falls back to `workspace_sync::restore_on_start`'s tar. The
/// shape rules match that function's: the mirror is a complete set of repos when
/// it carries the primary repo's `.git`, and paths are workspace-relative unless
/// the manifest came from the single-repo shape.
///
/// **Failure policy:** a failed transfer clears the workspace and returns
/// `None`, so the caller clones instead of running on half a tree.
pub(super) async fn restore_on_start(state: &SandboxState) -> Option<RestoreDecision> {
    let task_id = &state.config.task_id;
    let workspace_dir = PathBuf::from(&state.config.workspace_dir);
    let repo_dir = PathBuf::from(&state.config.repo_dir);
    let prefix = primary_prefix(&state.config.workspace_dir, &state.config.repo_dir);

    let started = Instant::now();
    let client = http_client(OBJECT_TIMEOUT).ok()?;
    let manifest = fetch_stored_manifest(task_id, state.backend_client.as_ref(), &client).await?;

    // Same three shapes the tar restore classifies, decided off the manifest
    // rather than by scanning an archive.
    let target = if manifest.carries_git(&prefix) {
        workspace_dir.clone()
    } else if manifest.carries_git("") {
        // A single-repo manifest written repo-relative belongs on `repo_dir`.
        repo_dir.clone()
    } else {
        warn!(task_id = %task_id, "Stored manifest carries no primary .git; falling back to the tar");
        return None;
    };

    let bytes = match download_manifest_files(state, &client, &manifest, &target).await {
        Ok(bytes) => bytes,
        Err(e) => {
            warn!(task_id = %task_id, error = %e, "Workspace mirror restore failed; clearing partial state and cloning");
            clear_dir_contents(&workspace_dir).await;
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
            return None;
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
    Some(RestoreDecision::RestoredSnapshot)
}

/// Fetch every file the manifest names into `target`, recreating symlinks from
/// the manifest rather than fetching them.
async fn download_manifest_files(
    state: &SandboxState,
    client: &reqwest::Client,
    manifest: &Manifest,
    target: &Path,
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

    for (chunk, urls) in batched_urls(
        state.backend_client.as_ref(),
        &state.config.task_id,
        &paths,
        "get",
        true,
    )
    .await?
    {
        let budget = Arc::new(Semaphore::new(INFLIGHT_BUDGET_MB));
        let transfers = chunk.into_iter().zip(urls).map(|(path, url)| {
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
                get_file(&client, &url, &full, entry.as_ref()).await
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

/// Mint presigned URLs for `paths` in batches the backend accepts, pairing each
/// batch with the paths it covers so a caller keeps the association.
async fn batched_urls(
    backend_client: &dyn BackendClient,
    task_id: &str,
    paths: &[String],
    operation: &str,
    restore_source: bool,
) -> Result<Vec<(Vec<String>, Vec<String>)>, String> {
    let mut batches = Vec::new();
    for chunk in paths.chunks(URL_BATCH) {
        let urls = backend_client
            .get_workspace_object_urls(task_id, chunk, operation, restore_source)
            .await
            .map_err(|e| format!("minting {operation} URLs failed: {e}"))?;
        batches.push((chunk.to_vec(), urls));
    }
    Ok(batches)
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
async fn put_file(client: &reqwest::Client, url: &str, path: &Path) -> Result<u64, String> {
    let body = match tokio::fs::read(path).await {
        Ok(body) => body,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("reading {} failed: {e}", path.display())),
    };
    let bytes = body.len() as u64;
    http_put(client, url, body).await?;
    Ok(bytes)
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
) -> Result<u64, String> {
    let body = http_get(client, url).await?.ok_or_else(|| {
        format!(
            "manifest names an object that is not stored: {}",
            path.display()
        )
    })?;
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

/// True when a gRPC error means workspace sync is unavailable rather than
/// failing. Mirrors `workspace_sync`'s own test.
fn is_feature_disabled(status: &tonic::Status) -> bool {
    matches!(
        status.code(),
        tonic::Code::FailedPrecondition | tonic::Code::Unimplemented
    )
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
