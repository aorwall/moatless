//! Snapshotting the workspace into the task's `restic` repository.
//!
//! One repository per task, at the prefix the backend names. A snapshot ships
//! only the blocks that moved, and it commits or it does not exist: a run
//! killed mid-write leaves unreferenced packs and no snapshot, never a tree
//! mixed from two generations. `rustic_core` is synchronous throughout, so
//! every call here runs inside one `spawn_blocking`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use aws_sdk_s3::config::retry::RetryConfig;
use aws_sdk_s3::config::timeout::TimeoutConfig;
use aws_sdk_s3::config::{Credentials, Region};
use moatless_s3::ResticS3Backend;
use rustic_core::repofile::SnapshotId;
use rustic_core::{
    BackupOptions, Credentials as RepoCredentials, ForgetGroups, Grouped, KeepOptions,
    LocalDestination, LsOptions, PathList, PruneOptions, Repository, RepositoryBackends,
    RepositoryOptions, RestoreOptions, SnapshotGroupCriterion, SnapshotOptions,
};
use sandbox_proto::SNAPSHOT_ROOT;
use tracing::{debug, info, warn};

use super::file_set;
use super::sync::{
    NotStored, RestoreAttempt, RestoreDecision, SyncOutcome, clear_dir_contents,
    is_feature_disabled, primary_repo_is_usable, report_skipped_unreadable_snapshot,
};
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::state::SandboxState;

/// The `format` label every transfer this module reports under.
pub(super) const FORMAT: &str = "restic";

/// Where the local pack cache lives.
///
/// Must be `/tmp`, never `no_cache` (`rustic_core` then issues one GET per
/// blob), `WORKSPACE_DIR` (feeds on itself), `/opt/moatless/data` (capped), or
/// `/run/moatless` (1Mi). `/tmp` is a `subPath` of the capped `workspace-data`
/// volume, so this spends the workspace's own eviction headroom.
const CACHE_DIR: &str = "/tmp/rustic-cache";

/// Where the whitelist of paths to snapshot is written.
///
/// Outside `WORKSPACE_DIR` on purpose: a file written into a checkout would
/// appear in the next snapshot.
const GLOB_FILE: &str = "/tmp/moatless-workspace-globs.txt";

/// The snapshot label, which must never vary.
///
/// Parent grouping is hostname + label + paths; a label that changed per
/// snapshot would find no parent and re-read every file as new (measured 1.92s
/// vs 546ms for the same delta).
const SNAPSHOT_LABEL: &str = "workspace";

/// Snapshots kept when a task prunes.
///
/// Enough to step back past a bad turn, few enough that a long-lived task does
/// not accumulate indefinitely. Scaled with `sync::DEFAULT_INTERVAL_SECS`: the
/// periodic loop is what writes most of these, so a count is only worth as much
/// history as the interval gives it.
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
        let credentials = Credentials::new(
            granted.access_key_id,
            granted.secret_access_key,
            Some(granted.session_token),
            None,
            "moatless-workspace",
        );
        // One client for both repositories: the session policy scopes the
        // credentials, and the prefix is what tells the two apart.
        let client = aws_sdk_s3::Client::from_conf(s3_config(granted.region, credentials));
        let access = Access {
            own: RepoAccess {
                backend: ResticS3Backend::new(
                    client.clone(),
                    granted.bucket.clone(),
                    &granted.prefix,
                ),
                password: granted.password,
            },
            source: match (granted.source_prefix, granted.source_password) {
                (Some(prefix), Some(password)) => Some(RepoAccess {
                    backend: ResticS3Backend::new(client, granted.bucket, &prefix),
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

/// How many times one S3 request is attempted before the operation carrying it
/// fails. Above `RetryConfig::standard()`'s own 3: a snapshot is hundreds of
/// requests and fails on the first one that runs out of attempts.
const MAX_ATTEMPTS: u32 = 5;

/// How long one attempt may spend reaching the endpoint.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// The client every repository operation speaks through.
///
/// Retries and connect timeout are explicit (`behavior_version_latest()`
/// disables retries today): unretried, a connect failure fails the whole
/// snapshot. No attempt timeout, since a pack upload is unbounded.
fn s3_config(region: String, credentials: Credentials) -> aws_sdk_s3::Config {
    aws_sdk_s3::Config::builder()
        .behavior_version_latest()
        .region(Region::new(region))
        .credentials_provider(credentials)
        .retry_config(RetryConfig::standard().with_max_attempts(MAX_ATTEMPTS))
        .timeout_config(
            TimeoutConfig::builder()
                .connect_timeout(CONNECT_TIMEOUT)
                .build(),
        )
        .build()
}

fn options() -> RepositoryOptions {
    RepositoryOptions::default().cache_dir(CACHE_DIR)
}

/// Write the whitelist that makes `rustic_core`'s walk yield today's file set.
///
/// Not `git_ignore`: that flag follows `.gitignore` file semantics, not the git
/// index, so it drops a file that is tracked **and** ignored (`git add -f`),
/// and restore then reports it deleted.
fn write_glob_file(workspace_dir: &Path) -> Result<usize, String> {
    let files = file_set::collect(workspace_dir);
    let mut out = String::new();
    let mut written = 0usize;
    for file in &files {
        let path = file.to_string_lossy();
        // A line-oriented glob file cannot carry either of these, and both are
        // legal on Linux. A newline splits one entry into two patterns; a
        // trailing space is stripped by the gitignore line parser, so the
        // pattern names a different file. Refusing is loud where mis-storing is
        // silent — a mis-stored path restores as a deleted file.
        if path.contains('\n') || path.ends_with(' ') {
            warn!(path = %path, "Path is not expressible as a glob; not snapshotted");
            continue;
        }
        out.push_str(&escape_glob(&path));
        out.push('\n');
        written += 1;
    }
    let mut handle = std::fs::File::create(GLOB_FILE).map_err(|e| e.to_string())?;
    handle
        .write_all(out.as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(written)
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
) -> Result<SyncOutcome, NotStored> {
    let task_id = config.task_id.clone();
    let workspace_dir = PathBuf::from(&config.workspace_dir);

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
    let outcome = tokio::task::spawn_blocking(move || snapshot(&access.own, &workspace_dir)).await;
    let elapsed = started.elapsed().as_millis() as u64;

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
fn snapshot(access: &RepoAccess, workspace_dir: &Path) -> Result<Option<u64>, String> {
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
    let repo = open(access)?
        .to_indexed_ids()
        .map_err(|e| format!("repository index is unusable: {e}"))?;

    let mut backup = BackupOptions::default();
    // The default sizes the source for a progress ETA before backing it up,
    // which is a second full walk. The whitelist cannot prune a directory —
    // `ignore` returns `Match::None` for one whatever the patterns say — so
    // both walks descend into `target/` and `node_modules/` regardless.
    backup.no_scan = true;
    backup.excludes.glob_files = vec![GLOB_FILE.to_string()];
    backup.as_path = Some(PathBuf::from(SNAPSHOT_ROOT));
    backup.ignore_filter_opts.git_ignore = false;
    backup.parent_opts.force = false;

    let paths = PathList::from_string(&workspace_dir.to_string_lossy())
        .map_err(|e| format!("workspace path is unusable: {e}"))?;
    let snapshot = SnapshotOptions::default()
        .label(Some(SNAPSHOT_LABEL.to_string()))
        .to_snapshot()
        .map_err(|e| format!("snapshot metadata is unusable: {e}"))?;

    let written = repo
        .backup(&backup, &paths, snapshot)
        .map_err(|e| format!("backup failed: {e}"))?;
    Ok(Some(written.summary.as_ref().map_or(0, |s| s.data_added)))
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
                restore(source, workspace_dir, unreadable)
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
    let repo = Repository::new(options, backends)
        .map_err(|e| format!("repository is unusable: {e}"))?;
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

    // `instant_delete`, because `PruneOptions` otherwise marks packs and removes
    // them 23 h later, which a task stopping right after this never reaches.
    // Measured: with the default, 42 packs / 1.35 GB stayed; with this, 12 packs
    // went and `check` returned to zero findings. Callers hold the upload
    // baseline across this, so no snapshot writes against the index it rewrites.
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
mod tests {
    use aws_sdk_s3::config::retry::RetryMode;
    use bytes::Bytes;

    use super::*;

    /// A filename holding a globset metacharacter must still name itself. A
    /// pattern that took `[` literally would silently drop every file it was
    /// meant to whitelist.
    #[test]
    fn a_metacharacter_in_a_filename_is_escaped_rather_than_matched() {
        assert_eq!(escape_glob("/w/repo/a[1].rs"), r"/w/repo/a\[1\].rs");
        assert_eq!(escape_glob("/w/repo/*.rs"), r"/w/repo/\*.rs");
        assert_eq!(escape_glob("/w/repo/{a,b}"), r"/w/repo/\{a,b\}");
        assert_eq!(escape_glob("/w/repo/back\\slash"), r"/w/repo/back\\slash");
        assert_eq!(escape_glob("/w/repo/plain.rs"), "/w/repo/plain.rs");
    }

    /// A connect that never completes is the fleet's dominant snapshot failure,
    /// and the SDK's own defaults move with its version, so the client states
    /// both itself.
    #[test]
    fn the_repository_client_retries_a_request_that_cannot_connect() {
        let config = s3_config(
            "eu-north-1".into(),
            Credentials::new("key", "secret", None, None, "test"),
        );
        assert_eq!(
            config.retry_config().map(RetryConfig::max_attempts),
            Some(MAX_ATTEMPTS)
        );
        // `adaptive()` would satisfy the count above and be a different client:
        // it adds a client-side rate limiter that throttles ahead of the request.
        assert_eq!(
            config.retry_config().map(RetryConfig::mode),
            Some(RetryMode::Standard)
        );
        assert_eq!(
            config
                .timeout_config()
                .and_then(TimeoutConfig::connect_timeout),
            Some(CONNECT_TIMEOUT)
        );
    }

    /// The glob file must not land in the tree it describes, or it would appear
    /// in the next snapshot and change the file set it defines.
    #[test]
    fn the_glob_file_and_the_cache_stay_out_of_the_workspace() {
        for path in [GLOB_FILE, CACHE_DIR] {
            assert!(path.starts_with("/tmp/"), "{path} is not under /tmp");
            assert!(!path.starts_with("/opt/moatless/workspace"), "{path}");
        }
    }

    // =========================================================================
    // Fidelity against a real S3 server
    // =========================================================================

    /// Every test that calls [`snapshot`] takes this for its whole body.
    ///
    /// [`GLOB_FILE`] is one fixed path, so overlapping snapshots in one test
    /// process let the second one's file set decide the first one's, and the
    /// failure reads as a file the restore lost.
    static SNAPSHOTTING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    /// Start the endpoint this needs, then run it:
    ///
    /// ```bash
    /// docker run -d --name minio-restic -p 19000:9000 \
    ///   -e MINIO_ROOT_USER=spikeadmin -e MINIO_ROOT_PASSWORD=spikeadmin123 \
    ///   minio/minio:latest server /data
    /// cargo test -p moatless-sandbox --lib restic -- --ignored --nocapture
    /// ```
    ///
    /// `git_ignore` cannot produce this file set: that flag is
    /// `.gitignore`-*file* semantics and not the git index, so a `git add -f`'d
    /// file is dropped and the restored repo reports it deleted. The tree below
    /// contains that case.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs an S3 endpoint at 127.0.0.1:19000"]
    async fn a_snapshot_restores_the_file_set_byte_for_byte() {
        use std::collections::BTreeMap;
        use std::os::unix::fs::PermissionsExt;

        let _snapshotting = SNAPSHOTTING.lock().await;

        let source = scratch("source");
        let restored = scratch("restored");
        build_fixture(&source);

        let access = minio_access(&format!("fidelity/{}", std::process::id())).await;
        let outcome = tokio::task::spawn_blocking({
            let source = source.clone();
            let restored = restored.clone();
            move || {
                let added = snapshot(&access, &source)?;
                let bytes = restore(&access, &restored, UnreadableSnapshot::Refuse)?
                    .map(|restored| restored.bytes);
                Ok::<_, String>((added, bytes))
            }
        })
        .await
        .expect("the snapshot task ran")
        .expect("the snapshot and restore succeeded");
        assert_eq!(
            outcome.0.map(|added| added > 0),
            Some(true),
            "a first snapshot adds data"
        );

        // The production walk is the reference: whatever it names is what the
        // snapshot had to carry.
        let expected: BTreeMap<String, std::fs::Metadata> = file_set::collect(&source)
            .into_iter()
            .filter_map(|file| {
                let meta = std::fs::symlink_metadata(&file).ok()?;
                Some((super::super::sync::rel_from(&source, &file), meta))
            })
            .collect();
        assert!(
            expected.keys().any(|p| p.contains("force-added.log")),
            "the fixture must contain a tracked-and-ignored file"
        );

        for (rel, meta) in &expected {
            let back = restored.join(rel);
            let Ok(back_meta) = std::fs::symlink_metadata(&back) else {
                panic!("{rel} did not come back");
            };
            if meta.is_symlink() {
                assert_eq!(
                    std::fs::read_link(source.join(rel)).ok(),
                    std::fs::read_link(&back).ok(),
                    "{rel} changed its link target"
                );
                continue;
            }
            assert_eq!(
                meta.permissions().mode() & 0o7777,
                back_meta.permissions().mode() & 0o7777,
                "{rel} changed its mode"
            );
            assert_eq!(
                std::fs::read(source.join(rel)).ok(),
                std::fs::read(&back).ok(),
                "{rel} changed its content"
            );
        }

        let actual = walk_paths(&restored);
        let extra: Vec<&String> = actual
            .iter()
            .filter(|p| !expected.contains_key(*p))
            .collect();
        assert!(
            extra.is_empty(),
            "the restore carries files the walk does not name: {extra:?}"
        );

        for repo in ["repo", "nested/sibling"] {
            assert_eq!(
                git(&source.join(repo), &["rev-parse", "HEAD"]),
                git(&restored.join(repo), &["rev-parse", "HEAD"]),
                "{repo} restored at a different commit"
            );
            assert_eq!(
                git(&source.join(repo), &["status", "--porcelain"]),
                git(&restored.join(repo), &["status", "--porcelain"]),
                "{repo} restored with a different working tree"
            );
            assert!(
                git(
                    &restored.join(repo),
                    &["fsck", "--no-progress", "--no-dangling"]
                )
                .is_some(),
                "{repo} restored a broken object database"
            );
        }

        _ = std::fs::remove_dir_all(&source);
        _ = std::fs::remove_dir_all(&restored);
    }

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "moatless-restic-{name}-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("scratch directory");
        path
    }

    /// A workspace holding every case the file set has to get right.
    fn build_fixture(root: &Path) {
        use std::os::unix::fs::PermissionsExt;

        let repo = root.join("repo");
        std::fs::create_dir_all(repo.join("src")).expect("repo");
        std::fs::write(repo.join(".gitignore"), "*.log\ntarget/\n").expect("gitignore");
        std::fs::write(repo.join("src/main.rs"), "fn main() {}").expect("source");
        std::fs::write(repo.join("AGENTS.md"), "guidance").expect("agents");
        // A filename holding a globset metacharacter.
        std::fs::write(repo.join("src/tuple[0].rs"), "type T = ();").expect("bracketed");
        std::os::unix::fs::symlink("AGENTS.md", repo.join("CLAUDE.md")).expect("symlink");
        std::fs::create_dir_all(repo.join("target")).expect("target");
        std::fs::write(repo.join("target/build.o"), vec![0u8; 64]).expect("build output");
        std::fs::write(repo.join("ignored.log"), "noise").expect("ignored");
        std::fs::write(repo.join("force-added.log"), "kept anyway").expect("forced");

        init_repo(&repo);
        git(&repo, &["add", "-A"]);
        // Tracked *and* matching `.gitignore`, which is the case `git_ignore`
        // gets wrong.
        git(&repo, &["add", "-f", "force-added.log"]);
        git(&repo, &["commit", "-m", "init"]);
        // Uncommitted work, which is what makes a snapshot better than a clone.
        std::fs::write(repo.join("src/main.rs"), "fn main() { todo!() }").expect("dirty");
        std::fs::write(repo.join("untracked.rs"), "// new").expect("untracked");

        let sibling = root.join("nested/sibling");
        std::fs::create_dir_all(&sibling).expect("sibling");
        std::fs::write(sibling.join("lib.rs"), "pub fn f() {}").expect("sibling source");
        init_repo(&sibling);
        git(&sibling, &["add", "-A"]);
        git(&sibling, &["commit", "-m", "init"]);

        // Outside any repo: kept whole, no gitignore to honor.
        std::fs::create_dir_all(root.join("notes")).expect("notes");
        std::fs::write(root.join("notes/plan.md"), "# plan").expect("note");
        let mut executable = std::fs::metadata(root.join("notes/plan.md"))
            .expect("note metadata")
            .permissions();
        executable.set_mode(0o755);
        std::fs::set_permissions(root.join("notes/plan.md"), executable).expect("mode");
    }

    fn init_repo(dir: &Path) {
        git(dir, &["init", "-q", "-b", "main"]);
        git(dir, &["config", "user.email", "test@example.com"]);
        git(dir, &["config", "user.name", "Test"]);
    }

    fn git(dir: &Path, args: &[&str]) -> Option<String> {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    }

    /// Every non-directory under `root`, workspace-relative.
    fn walk_paths(root: &Path) -> Vec<String> {
        let mut found = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for path in entries.flatten().map(|e| e.path()) {
                let Ok(meta) = std::fs::symlink_metadata(&path) else {
                    continue;
                };
                if meta.is_dir() {
                    stack.push(path);
                } else {
                    found.push(super::super::sync::rel_from(root, &path));
                }
            }
        }
        found
    }

    /// A fork's first start finds nothing under its own prefix, and the tree it
    /// must come up with is the source's. Ignored for the same reason as
    /// [`a_snapshot_restores_the_file_set_byte_for_byte`].
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs an S3 endpoint at 127.0.0.1:19000"]
    async fn a_fork_with_no_repository_of_its_own_restores_the_source_s() {
        let _snapshotting = SNAPSHOTTING.lock().await;

        let run = std::process::id();
        let source_tree = scratch("fork-source");
        let restored = scratch("fork-restored");
        std::fs::create_dir_all(source_tree.join("notes")).expect("the source tree is writable");
        std::fs::write(source_tree.join("notes/only-in-source.txt"), b"parent")
            .expect("the source file is writable");

        let source = minio_access(&format!("fork/{run}/source")).await;
        let fork = Access {
            // Never written in this test, so it stays a prefix holding nothing —
            // which is exactly a fork that has not snapshotted yet.
            own: minio_access(&format!("fork/{run}/own")).await,
            source: Some(source.clone()),
        };

        tokio::task::spawn_blocking({
            let source_tree = source_tree.clone();
            let restored = restored.clone();
            move || {
                snapshot(&source, &source_tree).expect("the source snapshotted");
                let own_only = Access {
                    own: fork.own.clone(),
                    source: None,
                };
                assert!(
                    restore_own_or_source(&own_only, &restored, UnreadableSnapshot::Refuse)
                        .expect("the fork's own repository")
                        .is_none(),
                    "the fork's own prefix holds no snapshot to fall back from"
                );
                restore_own_or_source(&fork, &restored, UnreadableSnapshot::Refuse)
                    .expect("the fork restored");
            }
        })
        .await
        .expect("the restore task ran");

        assert_eq!(
            std::fs::read(restored.join("notes/only-in-source.txt")).expect("the source's file"),
            b"parent"
        );
    }

    /// A repository on the local `MinIO`, addressed by a prefix of its own so
    /// repeated runs do not share state.
    async fn minio_access(prefix: &str) -> RepoAccess {
        let credentials = Credentials::new("spikeadmin", "spikeadmin123", None, None, "test");
        let config = aws_sdk_s3::Config::builder()
            .behavior_version_latest()
            .region(Region::new("us-east-1"))
            .credentials_provider(credentials)
            .endpoint_url("http://127.0.0.1:19000")
            .force_path_style(true)
            .build();
        let client = aws_sdk_s3::Client::from_conf(config);
        _ = client
            .create_bucket()
            .bucket("moatless-restic-test")
            .send()
            .await;
        RepoAccess {
            backend: ResticS3Backend::new(client, "moatless-restic-test", prefix),
            password: "fidelity".to_string(),
        }
    }

    /// A backend over a map, counting how many further reads of an object — and
    /// how many further snapshot listings — it refuses. Lets the tests below
    /// drive the real [`restore_from`] with no endpoint.
    #[derive(Default, Clone)]
    struct MapBackend {
        objects: Arc<std::sync::Mutex<std::collections::HashMap<(String, rustic_core::Id), Bytes>>>,
        refusals: Arc<std::sync::Mutex<std::collections::HashMap<rustic_core::Id, usize>>>,
        refused_lists: Arc<std::sync::Mutex<usize>>,
    }

    impl MapBackend {
        /// Refuse every later read of `id`, as a corrupt object does.
        fn make_unreadable(&self, id: rustic_core::Id) {
            self.refuse_reads(id, usize::MAX);
        }

        /// Refuse the next `count` reads of `id`, as a stream breaking mid-body
        /// does — the shape a later read clears.
        fn refuse_reads(&self, id: rustic_core::Id, count: usize) {
            _ = self
                .refusals
                .lock()
                .expect("map lock")
                .insert(id, count);
        }

        /// Refuse the next `count` snapshot listings. Snapshots only: opening the
        /// repository and loading the index list their own types first.
        fn refuse_snapshot_lists(&self, count: usize) {
            *self.refused_lists.lock().expect("map lock") = count;
        }

        /// Whether this read is refused, spending one of `id`'s refusals.
        fn refuses(&self, id: &rustic_core::Id) -> bool {
            let mut refusals = self.refusals.lock().expect("map lock");
            match refusals.get_mut(id) {
                Some(left) if *left > 0 => {
                    *left -= 1;
                    true
                }
                _ => false,
            }
        }
    }

    impl rustic_core::ReadBackend for MapBackend {
        fn location(&self) -> String {
            "map:".to_string()
        }

        fn list_with_size(
            &self,
            tpe: rustic_core::FileType,
        ) -> rustic_core::RusticResult<Vec<(rustic_core::Id, u32)>> {
            if tpe == rustic_core::FileType::Snapshot {
                let mut left = self.refused_lists.lock().expect("map lock");
                if *left > 0 {
                    *left -= 1;
                    return Err(rustic_core::RusticError::new(
                        rustic_core::ErrorKind::Backend,
                        "listing will not complete",
                    ));
                }
            }
            let dir = tpe.to_string();
            let objects = self.objects.lock().expect("map lock");
            Ok(objects
                .iter()
                .filter(|((t, _), _)| *t == dir)
                .map(|((_, id), body)| (*id, body.len() as u32))
                .collect())
        }

        fn read_full(
            &self,
            tpe: rustic_core::FileType,
            id: &rustic_core::Id,
        ) -> rustic_core::RusticResult<Bytes> {
            if self.refuses(id) {
                return Err(rustic_core::RusticError::new(
                    rustic_core::ErrorKind::Backend,
                    "object will not read",
                ));
            }
            let objects = self.objects.lock().expect("map lock");
            objects.get(&(tpe.to_string(), *id)).cloned().ok_or_else(|| {
                rustic_core::RusticError::new(rustic_core::ErrorKind::Backend, "no such object")
            })
        }

        fn read_partial(
            &self,
            tpe: rustic_core::FileType,
            id: &rustic_core::Id,
            _cacheable: bool,
            offset: u32,
            length: u32,
        ) -> rustic_core::RusticResult<Bytes> {
            let body = self.read_full(tpe, id)?;
            let start = offset as usize;
            Ok(body.slice(start..start + length as usize))
        }

        fn warmup_path(&self, tpe: rustic_core::FileType, id: &rustic_core::Id) -> String {
            format!("{tpe}/{id}")
        }
    }

    impl rustic_core::WriteBackend for MapBackend {
        fn write_bytes(
            &self,
            tpe: rustic_core::FileType,
            id: &rustic_core::Id,
            _cacheable: bool,
            content: rustic_core::BytesList,
        ) -> rustic_core::RusticResult<()> {
            let mut body = Vec::with_capacity(content.size());
            for chunk in content.slice() {
                body.extend_from_slice(chunk);
            }
            let mut objects = self.objects.lock().expect("map lock");
            _ = objects.insert((tpe.to_string(), *id), Bytes::from(body));
            Ok(())
        }

        fn remove(
            &self,
            tpe: rustic_core::FileType,
            id: &rustic_core::Id,
            _cacheable: bool,
        ) -> rustic_core::RusticResult<()> {
            let mut objects = self.objects.lock().expect("map lock");
            _ = objects.remove(&(tpe.to_string(), *id));
            Ok(())
        }
    }

    const MAP_PASSWORD: &str = "test-password";

    /// Caching off: a cached snapshot answers the read the backend refuses,
    /// which is the refusal every test below is about.
    fn map_options() -> RepositoryOptions {
        RepositoryOptions::default().no_cache(true)
    }

    fn map_backends(backend: &MapBackend) -> RepositoryBackends {
        RepositoryBackends::new(Arc::new(backend.clone()), None)
    }

    /// An initialized repository holding no snapshot.
    fn map_repo() -> MapBackend {
        let backend = MapBackend::default();
        _ = Repository::new(&map_options(), &map_backends(&backend))
            .expect("repository")
            .init(
                &RepoCredentials::password(MAP_PASSWORD),
                &Default::default(),
                &Default::default(),
            )
            .expect("init");
        backend
    }

    /// Snapshot `dir` into the repository and return the snapshot's id.
    fn map_snapshot(backend: &MapBackend, dir: &Path) -> rustic_core::Id {
        let repo = Repository::new(&map_options(), &map_backends(backend))
            .expect("repository")
            .open(&RepoCredentials::password(MAP_PASSWORD))
            .expect("open")
            .to_indexed_ids()
            .expect("index");

        let mut backup = BackupOptions::default();
        backup.as_path = Some(PathBuf::from(SNAPSHOT_ROOT));
        backup.parent_opts.force = true;

        let paths = PathList::from_string(&dir.to_string_lossy()).expect("paths");
        let snapshot = SnapshotOptions::default().to_snapshot().expect("metadata");
        *repo.backup(&backup, &paths, snapshot).expect("backup").id
    }

    fn write_tree(root: &Path, body: &str) {
        std::fs::create_dir_all(root).expect("mkdir");
        std::fs::write(root.join("a.txt"), body).expect("write");
    }

    fn map_restore(
        backend: &MapBackend,
        into: &Path,
        unreadable: UnreadableSnapshot,
    ) -> Result<Option<Restored>, String> {
        std::fs::create_dir_all(into).expect("mkdir");
        restore_from(
            &map_options(),
            &map_backends(backend),
            MAP_PASSWORD,
            into,
            unreadable,
        )
    }

    /// An empty repository is an absence, so the caller may descend to the tar.
    #[test]
    fn a_repository_holding_no_snapshot_is_an_absence() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let backend = map_repo();

        let restored = map_restore(
            &backend,
            &tmp.path().join("into"),
            UnreadableSnapshot::Refuse,
        )
        .expect("an empty repository is not a failure");

        assert!(restored.is_none(), "nothing stored, so nothing to restore");
    }

    /// A strict read fails on a listing before it decodes anything, so a listing
    /// that clears and proves the repository empty is still an absence. Reporting
    /// failure here would hold every upload the pod goes on to make.
    #[test]
    fn a_listing_that_clears_on_an_empty_repository_is_an_absence() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let backend = map_repo();

        backend.refuse_snapshot_lists(1);

        let restored = map_restore(
            &backend,
            &tmp.path().join("into"),
            UnreadableSnapshot::PassOver,
        )
        .expect("a listing that clears must not fail the attempt");

        assert!(
            restored.is_none(),
            "nothing is stored, so the caller clones with uploads flowing"
        );
    }

    /// The incident's shape at the snapshot level. `rustic_core` logs a snapshot
    /// object it cannot read and drops it while resolving `latest`, so without
    /// the strict read this restores the generation before it and says nothing.
    #[test]
    fn an_unreadable_newest_snapshot_fails_the_attempt() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let source = tmp.path().join("source");
        let backend = map_repo();

        write_tree(&source, "first");
        _ = map_snapshot(&backend, &source);
        write_tree(&source, "second");
        backend.make_unreadable(map_snapshot(&backend, &source));

        let refused = map_restore(
            &backend,
            &tmp.path().join("into"),
            UnreadableSnapshot::Refuse,
        );

        assert!(
            refused.is_err(),
            "a snapshot that will not read must reach the retry, not pass as an absence: {refused:?}"
        );
    }

    /// The last attempt, where refusing ends the task's durability for the pod's
    /// life. It restores the newest generation that does read, and says it did.
    #[test]
    fn the_last_attempt_skips_an_unreadable_snapshot_and_reports_it() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let source = tmp.path().join("source");
        let into = tmp.path().join("into");
        let backend = map_repo();

        write_tree(&source, "first");
        let before_readable = chrono::Utc::now();
        _ = map_snapshot(&backend, &source);
        let after_readable = chrono::Utc::now();
        // Two `Zoned::now()` stamps have to land apart for the window below to
        // separate the restored generation from the skipped one.
        std::thread::sleep(std::time::Duration::from_millis(20));
        write_tree(&source, "second");
        backend.make_unreadable(map_snapshot(&backend, &source));

        let restored = map_restore(&backend, &into, UnreadableSnapshot::PassOver)
            .expect("the readable generation is still restorable")
            .expect("a repository holding two snapshots is not an absence");

        assert!(
            restored.skipped_unreadable,
            "skipping a snapshot is the whole reason the task is told anything"
        );
        assert_eq!(
            std::fs::read_to_string(into.join("a.txt")).expect("restored file"),
            "first",
            "the generation before the unreadable one is what is left to restore"
        );
        // The restart note quotes this time, so it has to date the tree that is
        // actually on disk. The skipped snapshot is newer, so reporting its time
        // would tell the agent work reached a tree it never reached.
        let taken_at = restored.taken_at.expect("the restored snapshot is dated");
        assert!(
            taken_at >= before_readable && taken_at <= after_readable,
            "{taken_at} must fall in the restored generation's window \
             {before_readable}..{after_readable}, not the skipped newer one's"
        );
    }

    /// An unreadable *older* generation costs the task nothing: the newest still
    /// reads and is what gets restored. Reported all the same, because which
    /// generation would not read is not knowable — hence the wording of
    /// `report_skipped_unreadable_snapshot`, which claims no loss.
    #[test]
    fn an_unreadable_older_snapshot_still_restores_the_newest() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let source = tmp.path().join("source");
        let into = tmp.path().join("into");
        let backend = map_repo();

        write_tree(&source, "first");
        let oldest = map_snapshot(&backend, &source);
        write_tree(&source, "second");
        _ = map_snapshot(&backend, &source);
        backend.make_unreadable(oldest);

        let restored = map_restore(&backend, &into, UnreadableSnapshot::PassOver)
            .expect("the newest generation still reads")
            .expect("a repository holding two snapshots is not an absence");

        assert_eq!(
            std::fs::read_to_string(into.join("a.txt")).expect("restored file"),
            "second",
            "the newest generation reads, so nothing older may be restored over it"
        );
        assert!(
            restored.skipped_unreadable,
            "an object that will not read is still reported, without claiming loss"
        );
    }

    /// A read that breaks once and then works is the incident's own shape, and
    /// the strict read fails on it. Nothing was skipped, so nothing is reported.
    #[test]
    fn a_read_that_clears_on_the_second_pass_reports_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let source = tmp.path().join("source");
        let into = tmp.path().join("into");
        let backend = map_repo();

        write_tree(&source, "first");
        _ = map_snapshot(&backend, &source);
        write_tree(&source, "second");
        backend.refuse_reads(map_snapshot(&backend, &source), 1);

        let restored = map_restore(&backend, &into, UnreadableSnapshot::PassOver)
            .expect("the second read of the newest snapshot works")
            .expect("a repository holding two snapshots is not an absence");

        assert!(
            !restored.skipped_unreadable,
            "every stored snapshot read, so the task lost nothing to report"
        );
        assert_eq!(
            std::fs::read_to_string(into.join("a.txt")).expect("restored file"),
            "second",
            "the newest generation is what a cleared read restores"
        );
    }

    /// Nothing was skipped, so nothing is reported: no false report to make the
    /// real ones easy to dismiss.
    #[test]
    fn a_repository_that_reads_whole_reports_nothing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let source = tmp.path().join("source");
        let backend = map_repo();

        write_tree(&source, "first");
        _ = map_snapshot(&backend, &source);

        for unreadable in [UnreadableSnapshot::Refuse, UnreadableSnapshot::PassOver] {
            let restored = map_restore(&backend, &tmp.path().join("into"), unreadable)
                .expect("a whole repository restores")
                .expect("a stored snapshot is not an absence");
            assert!(
                !restored.skipped_unreadable,
                "{unreadable:?} restored the newest snapshot, so the task lost nothing"
            );
        }
    }
}
