//! Snapshotting the workspace into the task's `restic` repository.
//!
//! One repository per task, at the prefix the backend names. A snapshot ships
//! only the blocks that moved, and it commits or it does not exist: a run
//! killed mid-write leaves unreferenced packs and no snapshot, never a tree
//! mixed from two generations. `rustic_core` is synchronous throughout, so
//! every call here runs inside one `spawn_blocking`.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use moatless_s3::{ResticS3Backend, S3Credentials, S3Storage};
use rustic_core::repofile::{SnapshotFile, SnapshotId, StringList};
use rustic_core::{
    BackupOptions, Credentials as RepoCredentials, ForgetGroups, Grouped, KeepOptions,
    LocalDestination, LsOptions, PathList, PruneOptions, Repository, RepositoryBackends,
    RepositoryOptions, RestoreOptions, SnapshotGroupCriterion, SnapshotOptions,
};
use sandbox_proto::{RESTIC_CACHE_DIR, SNAPSHOT_ROOT};
use tracing::{debug, info, warn};

use super::file_set;
use super::phases::Phases;
use super::sync::{
    Capture, NotStored, RestoreAttempt, RestoreDecision, SyncOutcome, clear_dir_contents,
    fingerprint_now, is_feature_disabled, primary_repo_is_usable,
    report_skipped_unreadable_snapshot,
};
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::state::SandboxState;

/// The `format` label every transfer this module reports under.
pub(super) const FORMAT: &str = "restic";

/// [`RESTIC_CACHE_DIR`]. `/tmp` is a `subPath` of the capped `workspace-data`
/// volume, so the cache spends the workspace's own eviction headroom.
fn cache_dir() -> PathBuf {
    PathBuf::from(RESTIC_CACHE_DIR)
}

/// Where the whitelist of paths to snapshot is written.
///
/// Outside `WORKSPACE_DIR` on purpose: a file written into a checkout would
/// appear in the next snapshot.
fn glob_file() -> PathBuf {
    std::env::temp_dir().join("moatless-workspace-globs.txt")
}

/// The snapshot label, which must never vary.
///
/// Parent grouping is label + paths ([`backup_options`]); a label that changed per
/// snapshot would find no parent and re-read every file as new (measured 1.92s
/// vs 546ms for the same delta).
const SNAPSHOT_LABEL: &str = "workspace";

/// Tag marking the snapshot [`super::sync::flush_on_shutdown`] wrote, which
/// [`restore_from`] reads back to tell a final tree from a mid-turn capture.
///
/// Safe to vary per snapshot where [`SNAPSHOT_LABEL`] is not:
/// [`backup_options`] leaves `tags` out of parent grouping, so parent detection
/// ignores it and a tagged snapshot still finds its parent.
const SHUTDOWN_TAG: &str = "shutdown-flush";

/// Snapshots kept when a task prunes.
///
/// Enough to step back past a bad turn, bounded for a long-lived task. Scaled
/// with `config::DEFAULT_SYNC_INTERVAL_SECS`, since the periodic loop writes
/// most of them.
const KEEP_SNAPSHOTS: i32 = 50;

/// One repository: where its files are and what decrypts them.
#[derive(Clone)]
pub(super) struct RepoAccess {
    backend: ResticS3Backend,
    password: String,
}

/// The repositories one task's sync speaks to, as the backend granted them.
#[derive(Clone)]
pub(super) struct Access {
    /// The task's own repository. The only one anything writes.
    own: RepoAccess,
    /// The fork source's repository, granted only while this task holds none of
    /// its own.
    ///
    /// The grant's session policy allows no write there, so a snapshot aimed
    /// here would fail rather than overwrite the source.
    source: Option<RepoAccess>,
}

/// How long before its stated expiry a grant is replaced.
///
/// An operation holds the credentials it opened with for its whole run, and a
/// large snapshot runs for minutes, so this margin bounds an operation, not a
/// request.
const REFRESH_MARGIN_SECS: i64 = 300;

/// The grant every repository operation opens with, reused until close to
/// expiring.
///
/// A grant costs a backend round trip plus an STS mint and lasts an hour, so
/// minting one per sync wastes it; keyed by task id.
#[derive(Default)]
pub(super) struct AccessCache(tokio::sync::Mutex<Option<CachedAccess>>);

struct CachedAccess {
    task_id: String,
    access: Access,
    /// The grant's own `expires_at`. A grant stating none is used once and
    /// never held, so this is always a time the backend gave.
    expires_at: chrono::DateTime<chrono::Utc>,
}

impl AccessCache {
    /// The access the repository speaks through, minting a grant when none is
    /// held or the held one expires within [`REFRESH_MARGIN_SECS`].
    pub(super) async fn get(
        &self,
        task_id: &str,
        backend_client: &dyn BackendClient,
    ) -> Result<Access, tonic::Status> {
        // Held across the mint, so a turn boundary and a periodic tick that
        // arrive together take one grant between them rather than one each.
        let mut held = self.0.lock().await;
        let usable_until = chrono::Utc::now() + chrono::TimeDelta::seconds(REFRESH_MARGIN_SECS);
        if let Some(cached) = held.as_ref() {
            if cached.task_id == task_id && cached.expires_at > usable_until {
                return Ok(cached.access.clone());
            }
        }

        let granted = backend_client
            .get_workspace_repository_access(task_id)
            .await?;
        let expires_at = chrono::DateTime::parse_from_rfc3339(&granted.expires_at)
            .map(|at| at.with_timezone(&chrono::Utc));
        // One client for both repositories: the session policy scopes the
        // credentials, and the prefix is what tells the two apart.
        let storage = S3Storage::with_credentials(
            granted.region,
            S3Credentials {
                access_key_id: granted.access_key_id,
                secret_access_key: granted.secret_access_key,
                session_token: Some(granted.session_token),
            },
        );
        let access = Access {
            own: RepoAccess {
                backend: ResticS3Backend::new(
                    storage.clone(),
                    granted.bucket.clone(),
                    &granted.prefix,
                ),
                password: granted.password,
            },
            source: match (granted.source_prefix, granted.source_password) {
                (Some(prefix), Some(password)) => Some(RepoAccess {
                    backend: ResticS3Backend::new(storage, granted.bucket, &prefix),
                    password,
                }),
                (None, None) => None,
                // The two are independently `optional` on the wire, so a backend
                // that set one and not the other would otherwise read as a fork
                // whose fallback simply never fired.
                (prefix, _) => {
                    debug!(
                        task_id = %task_id,
                        has_prefix = prefix.is_some(),
                        "Workspace repository grant carries half a fork source; ignoring it"
                    );
                    None
                }
            },
        };

        *held = match expires_at {
            Ok(expires_at) => Some(CachedAccess {
                task_id: task_id.to_string(),
                access: access.clone(),
                expires_at,
            }),
            Err(error) => {
                debug!(task_id = %task_id, %error, "Repository access states no expiry; minting one per operation");
                None
            }
        };
        Ok(access)
    }

    /// Drop the held grant so the next operation mints one.
    ///
    /// Called by every operation that fails: credentials are one of the things
    /// that can fail an operation, and a session lasts an hour, so a grant kept
    /// past a failure would repeat it for the rest of that hour.
    pub(super) async fn invalidate(&self) {
        *self.0.lock().await = None;
    }
}

fn options() -> RepositoryOptions {
    RepositoryOptions::default().cache_dir(cache_dir())
}

/// Write the whitelist that makes `rustic_core`'s walk yield today's file set.
///
/// Not `git_ignore`: that flag follows `.gitignore` file semantics, not the git
/// index, so it drops a file that is tracked **and** ignored (`git add -f`),
/// and restore then reports it deleted.
fn write_glob_file(workspace_dir: &Path) -> Result<usize, String> {
    let set = file_set::collect(workspace_dir);
    let mut out = String::new();
    let mut written = 0usize;
    for file in &set.files {
        let Some(path) = glob_line(file) else {
            warn!(path = %file.display(), "Path is not expressible as a glob; not snapshotted");
            continue;
        };
        out.push_str(&path);
        out.push('\n');
        written += 1;
    }
    // The whitelist cannot prune a directory — `ignore` returns `Match::None`
    // for one whatever it says — so without these the walk descends into every
    // `target/` and `node_modules/` and stats each file there.
    let stored_dirs: HashSet<&Path> = set.files.iter().flat_map(|f| f.ancestors()).collect();
    for dir in &set.ignored_dirs {
        // A tracked file under an ignored directory (`git add -f`) still has to
        // be reached.
        if stored_dirs.contains(dir.as_path()) {
            continue;
        }
        if let Some(path) = glob_line(dir) {
            out.push('!');
            out.push_str(&path);
            out.push_str("/\n");
        }
    }
    let mut handle = std::fs::File::create(glob_file()).map_err(|e| e.to_string())?;
    handle
        .write_all(out.as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(written)
}

/// `path` as one literal glob-file line, or `None` when a line cannot carry it.
///
/// A newline splits one entry into two patterns; a trailing space is stripped
/// by the gitignore line parser, so the pattern names a different path. Both
/// are legal on Linux, and refusing is loud where mis-storing is silent.
fn glob_line(path: &Path) -> Option<String> {
    let path = path.to_string_lossy();
    if path.contains('\n') || path.ends_with(' ') {
        return None;
    }
    Some(escape_glob(&path))
}

/// A path becomes a literal glob only once globset's metacharacters are
/// escaped. Without this a filename holding one changes the pattern, and the
/// files it was meant to name go unstored.
fn escape_glob(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Snapshot the workspace, or report that it reached no repository.
///
/// `Err` means the caller must write the tar so the task keeps a durable copy.
pub(super) async fn upload(
    config: &Config,
    backend_client: &dyn BackendClient,
    cache: &AccessCache,
    capture: Capture,
    phases: &Arc<Phases>,
) -> Result<SyncOutcome, NotStored> {
    let task_id = config.task_id.clone();
    let workspace_dir = PathBuf::from(&config.workspace_dir);

    phases.enter("access");
    let access = match cache.get(&task_id, backend_client).await {
        Ok(access) => access,
        Err(status) if is_feature_disabled(&status) => {
            debug!(task_id = %task_id, %status, "This backend serves no workspace repository; falling back");
            return Err(NotStored::Absent);
        }
        Err(status) => {
            debug!(task_id = %task_id, %status, "No workspace repository access; falling back");
            return Err(NotStored::Failed);
        }
    };

    let started = Instant::now();
    let snapshot_phases = Arc::clone(phases);
    let outcome = tokio::task::spawn_blocking(move || {
        snapshot(&access.own, &workspace_dir, capture, &snapshot_phases)
    })
    .await;
    let elapsed = started.elapsed().as_millis() as u64;
    phases.enter("report");

    let result = match outcome {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(error)) => Err(error),
        Err(error) => Err(format!("snapshot panicked: {error}")),
    };

    match result {
        Ok(None) => {
            debug!(task_id = %task_id, "The workspace holds no files to snapshot");
            Ok(SyncOutcome::default())
        }
        Ok(Some(bytes)) => {
            debug!(task_id = %task_id, bytes, "Snapshotted the workspace");
            backend_client
                .report_workspace_sync(&task_id, "upload", FORMAT, bytes, elapsed, None)
                .await;
            // `SyncWorkspaceResponse.synced` means bytes moved, not that a
            // snapshot committed: a snapshot of an unchanged tree adds none,
            // and the pre-stop caller logs a push off this flag.
            Ok(SyncOutcome {
                synced: bytes > 0,
                bytes,
            })
        }
        Err(error) => {
            warn!(task_id = %task_id, %error, "Workspace snapshot failed (non-fatal)");
            cache.invalidate().await;
            backend_client
                .report_workspace_sync(&task_id, "upload", FORMAT, 0, elapsed, Some(&error))
                .await;
            Err(NotStored::Failed)
        }
    }
}

/// One snapshot, start to finish, on a blocking thread. Returns the bytes the
/// snapshot added to the repository.
fn snapshot(
    access: &RepoAccess,
    workspace_dir: &Path,
    capture: Capture,
    phases: &Phases,
) -> Result<Option<u64>, String> {
    phases.enter("file set");
    let check = FinalTreeCheck::start(workspace_dir, capture);
    let count = write_glob_file(workspace_dir)?;
    // Not an error: the mirror answers the same state with an empty delta, and
    // reporting a failed `upload` here would put a permanent error-rate gap
    // between the two series during the soak.
    if count == 0 {
        return Ok(None);
    }

    // Reopened per snapshot, never reused: the parent snapshot is resolved
    // against the index the handle was opened with, so a handle held across two
    // snapshots cannot see the first one's tree and re-reads every file as new.
    phases.enter("index");
    let repo = open(access)?
        .to_indexed_ids()
        .map_err(|e| format!("repository index is unusable: {e}"))?;

    let backup = backup_options();
    let paths = PathList::from_string(&workspace_dir.to_string_lossy())
        .map_err(|e| format!("workspace path is unusable: {e}"))?;
    let snapshot = SnapshotOptions::default()
        .label(Some(SNAPSHOT_LABEL.to_string()))
        .to_snapshot()
        .map_err(|e| format!("snapshot metadata is unusable: {e}"))?;

    phases.enter("backup");
    let written = repo
        .backup(&backup, &paths, snapshot)
        .map_err(|e| format!("backup failed: {e}"))?;
    phases.enter("tag");
    if check.holds(workspace_dir) {
        certify_final_tree(&repo, &written);
    }
    Ok(Some(written.summary.as_ref().map_or(0, |s| s.data_added)))
}

/// How every snapshot walks the workspace: through [`glob_file`] alone.
fn backup_options() -> BackupOptions {
    let mut backup = BackupOptions::default();
    // The default sizes the source for a progress ETA before backing it up,
    // which is a second full walk.
    backup.no_scan = true;
    backup.excludes.glob_files = vec![glob_file().display().to_string()];
    backup.as_path = Some(PathBuf::from(SNAPSHOT_ROOT));
    backup.ignore_filter_opts.git_ignore = false;
    backup.parent_opts.force = false;
    // Not the default, which adds hostname: that is the pod's name, so each
    // pod's first snapshot would find no parent and re-read the whole restored
    // tree, long enough to run a shutdown flush out of its budget.
    backup.parent_opts.group_by = Some(SnapshotGroupCriterion::new().label(true).paths(true));
    // Restore keeps size and mtime but cannot keep ctime, so a ctime check
    // reads every restored file as changed.
    backup.parent_opts.ignore_ctime = true;
    backup
}

/// A last denial of the final-tree tag, over the tree the capture stored.
///
/// Metadata only: a change denies the tag and a match proves nothing, so no
/// part of the claim rests here. `workspace_writers_are_quiet` is what grants
/// it, over writers this pod either excluded or observed.
struct FinalTreeCheck {
    before: Option<u64>,
}

impl FinalTreeCheck {
    /// Read the tree before anything fixes what the capture will contain.
    ///
    /// A file created after this is in neither [`glob_file`] nor the snapshot, so
    /// a baseline taken any later would match on both sides and certify a tree
    /// missing it.
    fn start(workspace_dir: &Path, capture: Capture) -> Self {
        Self {
            before: capture
                .may_certify_final_tree()
                .then(|| fingerprint_now(workspace_dir))
                .flatten(),
        }
    }

    /// Whether the capture that has just been stored may carry the tag.
    fn holds(&self, workspace_dir: &Path) -> bool {
        self.before.is_some() && fingerprint_now(workspace_dir) == self.before
    }
}

/// Tag a stored snapshot as the tree its pod ended with.
///
/// `restic` cannot rewrite a snapshot in place, so the tagged copy is saved and
/// the untagged one deleted. Every failure is logged and swallowed: the tree is
/// already stored, and an uncertified one only costs the restart a dated note.
fn certify_final_tree<S: rustic_core::Open>(repo: &Repository<S>, written: &SnapshotFile) {
    let Ok(tag) = StringList::from_str(SHUTDOWN_TAG) else {
        return;
    };
    let mut tagged = written.clone();
    if !tagged.add_tags(vec![tag]) {
        return;
    }
    if let Err(error) = repo.save_snapshots(vec![tagged]) {
        warn!(%error, "Could not tag the shutdown flush as the final tree");
        return;
    }
    // Both snapshots carry the same tree, so a failure here leaves the restore
    // choosing between two readable generations of one capture.
    if let Err(error) = repo.delete_snapshots(&[written.id]) {
        warn!(%error, "Left the untagged copy of the shutdown flush in place");
    }
}

/// Open the repository, creating it when the task has none yet.
fn open(access: &RepoAccess) -> Result<Repository<rustic_core::OpenStatus>, String> {
    let backends = RepositoryBackends::new(Arc::new(access.backend.clone()), None);
    let credentials = RepoCredentials::password(&access.password);
    let repo = Repository::new(&options(), &backends)
        .map_err(|e| format!("repository is unusable: {e}"))?;

    if repo
        .config_id()
        .map_err(|e| format!("repository config is unreadable: {e}"))?
        .is_none()
    {
        // `init` and `open` each consume the handle, so this needs its own.
        let fresh = || {
            Repository::new(&options(), &backends)
                .map_err(|e| format!("repository is unusable: {e}"))
        };
        _ = fresh()?
            .init(&credentials, &Default::default(), &Default::default())
            .map_err(|e| format!("repository could not be created: {e}"))?;
        return fresh()?
            .open(&credentials)
            .map_err(|e| format!("new repository will not open: {e}"));
    }

    repo.open(&credentials)
        .map_err(|e| format!("repository will not open: {e}"))
}

/// How many times the whole restore is attempted before the snapshot is
/// called unreadable.
///
/// [`RestoreAttempt::Failed`] ends the task's durability for the pod's life,
/// so nothing transient may reach it. Covers what a per-read retry cannot:
/// a cancelled grant, an outage spanning seconds.
pub(super) const RESTORE_ATTEMPTS: u32 = 3;

/// Wait before the next whole-restore attempt, doubled per attempt.
///
/// Spent on the startup path and only after a failure, against a hold that
/// costs the task every byte it produces afterwards.
const RESTORE_RETRY_BACKOFF: Duration = Duration::from_secs(2);

/// What one attempt does about a stored snapshot object it cannot read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnreadableSnapshot {
    /// Fail the attempt, so the retry re-tests what is usually a blip.
    Refuse,
    /// Restore the newest generation that does read, and say so.
    PassOver,
}

/// A restored workspace.
#[derive(Debug)]
struct Restored {
    bytes: u64,
    /// A stored snapshot object would not read and was left out of the pick. Its
    /// time is inside the object that will not read, so whether it was newer
    /// than the snapshot restored here is not knowable.
    skipped_unreadable: bool,
    /// When the restored snapshot started — the tree's age, not a bound on what
    /// reached it. `None` only when the snapshot's own time will not convert,
    /// which the restart note words as unknown, not as current.
    taken_at: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether the restored snapshot carries [`SHUTDOWN_TAG`].
    final_flush: bool,
}

/// Restore the whole workspace from the latest snapshot.
///
/// [`RestoreAttempt::Restored`] means the caller can skip the clone: a snapshot
/// carries every repo's `.git`, reproducing HEAD, index, branch and dirty tree
/// exactly. `Absent` sends the caller to the next format; `Failed` does not.
pub(super) async fn restore_on_start(state: &SandboxState) -> RestoreAttempt {
    let task_id = state.config.task_id.clone();
    let workspace_dir = PathBuf::from(&state.config.workspace_dir);
    let repo_dir = PathBuf::from(&state.config.repo_dir);

    let started = Instant::now();
    let mut backoff = RESTORE_RETRY_BACKOFF;
    let mut attempts_left = RESTORE_ATTEMPTS;
    let restored = loop {
        attempts_left -= 1;
        // The last attempt passes over a snapshot object that still will not
        // read, so a repository holding one restores its newest readable
        // generation rather than ending the task's durability.
        let unreadable = if attempts_left == 0 {
            UnreadableSnapshot::PassOver
        } else {
            UnreadableSnapshot::Refuse
        };
        match restore_once(state, unreadable).await {
            Ok(restored) => break Ok(restored),
            Err(error) => {
                // A snapshot carries the whole workspace — every repo,
                // `uploads/`, loose directories — so a half-finished restore
                // leaves partial files across all of it, not only under
                // `repo_dir`, and the next attempt must not read them.
                clear_dir_contents(&workspace_dir).await;
                // The grant is one of the things that can fail an attempt and it
                // is held for an hour, so a retry keeping it would repeat the
                // failure rather than test it.
                state.workspace_sync.repository_access.invalidate().await;
                if attempts_left == 0 {
                    break Err(error);
                }
                warn!(task_id = %task_id, %error, attempts_left, "Restoring the workspace repository failed; retrying");
                tokio::time::sleep(backoff).await;
                backoff *= 2;
            }
        }
    };
    let elapsed = started.elapsed().as_millis() as u64;

    match restored {
        Ok(Some(restored)) => {
            info!(task_id = %task_id, bytes = restored.bytes, dir = %workspace_dir.display(), "Restored repos from the workspace repository (skipping clone)");
            state
                .backend_client
                .report_workspace_sync(&task_id, "download", FORMAT, restored.bytes, elapsed, None)
                .await;
            if restored.skipped_unreadable {
                report_skipped_unreadable_snapshot(state);
            }
            // A snapshot that turned out not to carry the primary repo is not
            // one a clone can be skipped after.
            if primary_repo_is_usable(&repo_dir).await {
                RestoreAttempt::Restored(RestoreDecision::RestoredSnapshot {
                    taken_at: restored.taken_at,
                    final_flush: restored.final_flush,
                })
            } else {
                debug!(task_id = %task_id, "Restored tree has no usable primary repo; will clone");
                RestoreAttempt::Absent
            }
        }
        Ok(None) => {
            debug!(task_id = %task_id, "No stored workspace snapshot; will try the mirror");
            RestoreAttempt::Absent
        }
        Err(error) => {
            warn!(task_id = %task_id, %error, attempts = RESTORE_ATTEMPTS, "Restoring the workspace repository failed (non-fatal); partial state cleared");
            state
                .backend_client
                .report_workspace_sync(&task_id, "download", FORMAT, 0, elapsed, Some(&error))
                .await;
            RestoreAttempt::Failed
        }
    }
}

/// One restore attempt: `Ok(None)` means this task has no snapshot to restore,
/// which no retry can change.
async fn restore_once(
    state: &SandboxState,
    unreadable: UnreadableSnapshot,
) -> Result<Option<Restored>, String> {
    let task_id = state.config.task_id.clone();
    let workspace_dir = PathBuf::from(&state.config.workspace_dir);

    let access = match state
        .workspace_sync
        .repository_access
        .get(&task_id, state.backend_client.as_ref())
        .await
    {
        Ok(access) => access,
        // Only the backend saying it mints no repository access at all is an
        // absence. A blip here carries no verdict about whether this task has a
        // repository, and reading one as "none" is what sends a restic-only
        // task to the rescue tar.
        Err(status) if is_feature_disabled(&status) => {
            debug!(task_id = %task_id, %status, "No workspace repository access; will try the mirror");
            return Ok(None);
        }
        Err(status) => return Err(format!("repository access could not be minted: {status}")),
    };

    match tokio::task::spawn_blocking(move || {
        restore_own_or_source(&access, &workspace_dir, unreadable)
    })
    .await
    {
        Ok(restored) => restored,
        Err(error) => Err(format!("restore panicked: {error}")),
    }
}

/// Restore from the task's own repository, falling back to the fork source's
/// when the task's own holds no snapshot.
///
/// Safe: the read-only source can't be damaged by a wrongly-fired fallback.
fn restore_own_or_source(
    access: &Access,
    workspace_dir: &Path,
    unreadable: UnreadableSnapshot,
) -> Result<Option<Restored>, String> {
    match restore(&access.own, workspace_dir, unreadable)? {
        Some(restored) => Ok(Some(restored)),
        None => match access.source.as_ref() {
            Some(source) => {
                info!("Restoring from the fork source's workspace repository");
                // The source's tag certifies the parent task's shutdown, never
                // this one's: a fork whose first pod died mid-turn restores the
                // parent tree, and its own interrupted work is simply absent.
                Ok(
                    restore(source, workspace_dir, unreadable)?.map(|restored| Restored {
                        final_flush: false,
                        ..restored
                    }),
                )
            }
            None => Ok(None),
        },
    }
}

/// Restore the latest snapshot onto `workspace_dir`, or `Ok(None)` when the
/// repository holds none.
fn restore(
    access: &RepoAccess,
    workspace_dir: &Path,
    unreadable: UnreadableSnapshot,
) -> Result<Option<Restored>, String> {
    let backends = RepositoryBackends::new(Arc::new(access.backend.clone()), None);
    restore_from(
        &options(),
        &backends,
        &access.password,
        workspace_dir,
        unreadable,
    )
}

/// [`restore`] over any backend, so a test can drive it without S3.
fn restore_from(
    options: &RepositoryOptions,
    backends: &RepositoryBackends,
    password: &str,
    workspace_dir: &Path,
    unreadable: UnreadableSnapshot,
) -> Result<Option<Restored>, String> {
    let credentials = RepoCredentials::password(password);
    let repo =
        Repository::new(options, backends).map_err(|e| format!("repository is unusable: {e}"))?;
    if repo
        .config_id()
        .map_err(|e| format!("repository config is unreadable: {e}"))?
        .is_none()
    {
        return Ok(None);
    }
    let repo = repo
        .open(&credentials)
        .map_err(|e| format!("repository will not open: {e}"))?
        .to_indexed()
        .map_err(|e| format!("repository index is unusable: {e}"))?;

    // `get_all_snapshots` fails on a snapshot object it cannot read, where
    // resolving `latest` logs and drops it (`snapshotfile.rs:892`). Only the
    // strict read tells an empty repository from one a blip hid a generation
    // of, so the newest is picked here rather than by `latest:`.
    let (snapshot, skipped_unreadable) = match repo.get_all_snapshots() {
        Ok(stored) => {
            let Some(snapshot) = stored.into_iter().max_by(|a, b| a.time.cmp(&b.time)) else {
                debug!("Workspace repository holds no snapshot yet");
                return Ok(None);
            };
            (snapshot, false)
        }
        Err(error) if unreadable == UnreadableSnapshot::Refuse => {
            return Err(format!("a stored snapshot could not be read: {error}"));
        }
        Err(error) => {
            // The listing decodes nothing and the filter runs once per object
            // that did decode, so the two counts say whether the tolerant read
            // dropped anything — the failure above may have been one bad read
            // that this second pass gets. Which object it dropped stays unknown.
            let stored = repo
                .list::<SnapshotId>()
                .map_err(|e| format!("snapshots are unlistable: {e}"))?
                .count();
            // `get_all_snapshots` lists before it decodes anything
            // (`update_from_backend`), so its failure may be the listing rather
            // than an object. A repository this one proves empty is an absence:
            // `latest` would call it a failure, which holds every upload the
            // pod goes on to make.
            if stored == 0 {
                debug!(%error, "Workspace repository holds no snapshot yet");
                return Ok(None);
            }
            let mut decoded = 0_usize;
            let snapshot = repo
                .get_snapshot_from_str("latest", |_| {
                    decoded += 1;
                    true
                })
                .map_err(|e| format!("no stored snapshot could be read: {e}"))?;
            if decoded < stored {
                warn!(%error, stored, decoded, "A stored snapshot will not read; restoring the newest that will");
            } else {
                info!(%error, "A stored snapshot failed one read and read on the retry");
            }
            (snapshot, decoded < stored)
        }
    };

    let taken_at = snapshot_taken_at(&snapshot);
    // A skipped generation may be newer than this one, so a tag here certifies
    // only the tree it was written for, not the tree the pod ended with.
    let final_flush = !skipped_unreadable && snapshot.tags.contains(SHUTDOWN_TAG);
    let node = repo
        .node_from_snapshot_and_path(&snapshot, SNAPSHOT_ROOT)
        .map_err(|e| format!("snapshot {} carries no `{SNAPSHOT_ROOT}`: {e}", snapshot.id))?;

    let destination = LocalDestination::new(&format!("{}/", workspace_dir.display()), true, false)
        .map_err(|e| format!("workspace is not writable: {e}"))?;
    let streamer = repo
        .ls(&node, &LsOptions::default())
        .map_err(|e| format!("snapshot tree is unwalkable: {e}"))?;
    let restore_opts = RestoreOptions::default();
    let plan = repo
        .prepare_restore(&restore_opts, streamer.clone(), &destination, false)
        .map_err(|e| format!("restore could not be planned: {e}"))?;
    let bytes = plan.restore_size;
    repo.restore(plan, &restore_opts, streamer, &destination)
        .map_err(|e| format!("restore failed: {e}"))?;
    Ok(Some(Restored {
        bytes,
        skipped_unreadable,
        taken_at,
        final_flush,
    }))
}

/// When a snapshot started, as UTC — its age, not a bound on what reached it.
///
/// `SnapshotFile::time` is stamped by `to_snapshot` after [`snapshot`] has
/// already enumerated paths, so it is the start and not `summary.backup_end`.
/// `jiff` is `rustic_core`'s clock type and `chrono` is this crate's, so the
/// conversion goes through the epoch rather than a formatted string.
fn snapshot_taken_at(
    snapshot: &rustic_core::repofile::SnapshotFile,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let timestamp = snapshot.time.timestamp();
    chrono::DateTime::from_timestamp(timestamp.as_second(), timestamp.subsec_nanosecond() as u32)
}

/// Drop all but the last [`KEEP_SNAPSHOTS`] snapshots and reclaim their packs.
///
/// Rewrites the index, so callers space it out — `sync::PRUNE_EVERY_SNAPSHOTS`
/// and the pre-stop RPC. Best-effort — a repository that keeps a few too many
/// still restores.
pub(super) async fn forget_and_prune(
    config: &Config,
    backend_client: &dyn BackendClient,
    cache: &AccessCache,
) {
    let task_id = config.task_id.clone();
    let Ok(access) = cache.get(&task_id, backend_client).await else {
        return;
    };
    match tokio::task::spawn_blocking(move || prune(&access.own)).await {
        Ok(Ok(forgotten)) => {
            debug!(task_id = %task_id, forgotten, "Pruned the workspace repository");
        }
        Ok(Err(error)) => {
            warn!(task_id = %task_id, %error, "Pruning the workspace repository failed (non-fatal)");
            cache.invalidate().await;
        }
        Err(error) => {
            warn!(task_id = %task_id, %error, "Workspace prune panicked (non-fatal)");
            cache.invalidate().await;
        }
    }
}

fn prune(access: &RepoAccess) -> Result<usize, String> {
    let repo = open(access)?;
    let snapshots = repo
        .get_all_snapshots()
        .map_err(|e| format!("snapshots are unreadable: {e}"))?;
    let grouped = Grouped::from_items(snapshots, SnapshotGroupCriterion::new());
    let groups = ForgetGroups::from_grouped_snapshots_with_retention(
        grouped,
        &KeepOptions::default().keep_last(KEEP_SNAPSHOTS),
        &rustic_core::jiff::Zoned::now(),
    )
    .map_err(|e| format!("retention could not be applied: {e}"))?;
    let forget = groups.into_forget_ids();
    if !forget.is_empty() {
        repo.delete_snapshots(&forget)
            .map_err(|e| format!("snapshots could not be deleted: {e}"))?;
    }

    // `instant_delete`, because `PruneOptions` otherwise marks packs and
    // removes them 23 h later, which a short-lived task never reaches.
    // Callers hold the upload baseline across this call, so no snapshot
    // write races the index it rewrites.
    let mut opts = PruneOptions::default();
    opts.instant_delete = true;
    let plan = repo
        .prune_plan(&opts)
        .map_err(|e| format!("prune could not be planned: {e}"))?;
    repo.prune(&opts, plan)
        .map_err(|e| format!("prune failed: {e}"))?;
    Ok(forget.len())
}

#[cfg(test)]
mod tests;
