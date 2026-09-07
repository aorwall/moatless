//! Snapshotting the workspace into the task's `restic` repository.
//!
//! One repository per task, at the prefix the backend names, opened with the
//! password it derives. A snapshot ships only the blocks that moved, and it
//! commits or it does not exist: a run killed mid-write leaves unreferenced
//! packs and no snapshot, never a tree mixed from two generations.
//!
//! `rustic_core` is synchronous throughout, so every call here runs inside one
//! `spawn_blocking`.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use aws_sdk_s3::config::{Credentials, Region};
use moatless_s3::ResticS3Backend;
use rustic_core::{
    BackupOptions, Credentials as RepoCredentials, ForgetGroups, Grouped, KeepOptions,
    LocalDestination, LsOptions, PathList, PruneOptions, Repository, RepositoryBackends,
    RepositoryOptions, RestoreOptions, SnapshotGroupCriterion, SnapshotOptions,
};
use sandbox_proto::SNAPSHOT_ROOT;
use tracing::{debug, info, warn};

use super::file_set;
use super::sync::{RestoreDecision, SyncOutcome, clear_dir_contents, primary_repo_is_usable};
use crate::backend_client::BackendClient;
use crate::config::Config;
use crate::state::SandboxState;

/// The `format` label every transfer this module reports under.
pub(super) const FORMAT: &str = "restic";

/// Where the local pack cache lives.
///
/// Mandatory, never `no_cache`. Without it `rustic_core` issues one range GET
/// per blob: measured in-cluster, a tree listing goes from 628 ms to 15.6 s and
/// a restore from 13 requests to 1,222.
///
/// `/tmp` and not somewhere under `WORKSPACE_DIR`: the workspace is the snapshot
/// source and a cache there would feed on itself. Not `/opt/moatless/data` (the
/// capped `shared-data` emptyDir every sidecar mounts) and not `/run/moatless`
/// (`medium: Memory`, `sizeLimit: 1Mi`, which cannot hold a cache at all).
///
/// `pod_builder.rs` serves `/tmp` from the `workspace-data` volume's `tmp`
/// subPath, so this spends the budget whose overflow evicts the pod, and
/// nothing bounds it — see `docs/deferred-changes.md`.
const CACHE_DIR: &str = "/tmp/rustic-cache";

/// Where the whitelist of paths to snapshot is written.
///
/// Outside `WORKSPACE_DIR` on purpose: the sandbox must not write into a
/// checkout, and a file written there would appear in the next snapshot.
const GLOB_FILE: &str = "/tmp/moatless-workspace-globs.txt";

/// The snapshot label, which must never vary.
///
/// The default parent grouping is hostname + label + paths. A label that
/// changed per snapshot would find no parent and re-read every file as new:
/// measured at 1.92 s against 546 ms for the same delta.
const SNAPSHOT_LABEL: &str = "workspace";

/// Snapshots kept when a stopping task prunes.
///
/// Enough that a restore can step back past a bad turn, few enough that a
/// long-lived task does not accumulate indefinitely.
const KEEP_SNAPSHOTS: i32 = 10;

/// Everything needed to open one task's repository, as the backend granted it.
struct Access {
    backend: ResticS3Backend,
    password: String,
}

/// Fetch access and build the S3 client the repository speaks through.
///
/// Fetched per sync rather than cached: a sync happens minutes apart at most, a
/// session lasts an hour, and refetching removes the whole class of bug where a
/// snapshot fails on an expired token the sandbox was still holding.
async fn access(
    task_id: &str,
    backend_client: &dyn BackendClient,
) -> Result<Access, tonic::Status> {
    let granted = backend_client
        .get_workspace_repository_access(task_id)
        .await?;
    let credentials = Credentials::new(
        granted.access_key_id,
        granted.secret_access_key,
        Some(granted.session_token),
        None,
        "moatless-workspace",
    );
    let config = aws_sdk_s3::Config::builder()
        .behavior_version_latest()
        .region(Region::new(granted.region))
        .credentials_provider(credentials)
        .build();
    Ok(Access {
        backend: ResticS3Backend::new(
            aws_sdk_s3::Client::from_conf(config),
            granted.bucket,
            &granted.prefix,
        ),
        password: granted.password,
    })
}

fn options() -> RepositoryOptions {
    RepositoryOptions::default().cache_dir(CACHE_DIR)
}

/// Write the whitelist that makes `rustic_core`'s walk yield today's file set.
///
/// A whitelist and not `git_ignore`: that flag is `.gitignore`-*file* semantics
/// and not the git index, so it drops a file that is tracked **and** ignored
/// (`git add -f`) and the restore then reports it deleted.
fn write_glob_file(workspace_dir: &Path) -> Result<usize, String> {
    let files = file_set::collect(workspace_dir);
    let mut out = String::new();
    let mut written = 0usize;
    for file in &files {
        let path = file.full.to_string_lossy();
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

/// Snapshot the workspace, or report that no repository could be reached.
///
/// `Err(())` means this tree reached no repository — the backend grants none, or
/// the transfer failed — and the caller must write another format so the task
/// keeps a durable copy.
pub(super) async fn upload(
    config: &Config,
    backend_client: &dyn BackendClient,
) -> Result<SyncOutcome, ()> {
    let task_id = config.task_id.clone();
    let workspace_dir = PathBuf::from(&config.workspace_dir);

    let access = match access(&task_id, backend_client).await {
        Ok(access) => access,
        Err(status) => {
            debug!(task_id = %task_id, %status, "No workspace repository access; falling back");
            return Err(());
        }
    };

    let started = Instant::now();
    let outcome = tokio::task::spawn_blocking(move || snapshot(&access, &workspace_dir)).await;
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
            backend_client
                .report_workspace_sync(&task_id, "upload", FORMAT, 0, elapsed, Some(&error))
                .await;
            Err(())
        }
    }
}

/// One snapshot, start to finish, on a blocking thread. Returns the bytes the
/// snapshot added to the repository.
fn snapshot(access: &Access, workspace_dir: &Path) -> Result<Option<u64>, String> {
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
fn open(access: &Access) -> Result<Repository<rustic_core::OpenStatus>, String> {
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

/// Restore the whole workspace from the latest snapshot.
///
/// `Some(RestoredSnapshot)` means the caller can skip the clone: a snapshot
/// carries every repo's `.git`, so it reproduces HEAD, the index, the branch and
/// the dirty tree exactly. `None` sends the caller on to the next format.
pub(super) async fn restore_on_start(state: &SandboxState) -> Option<RestoreDecision> {
    let task_id = state.config.task_id.clone();
    let workspace_dir = PathBuf::from(&state.config.workspace_dir);
    let repo_dir = PathBuf::from(&state.config.repo_dir);

    let access = match access(&task_id, state.backend_client.as_ref()).await {
        Ok(access) => access,
        Err(status) => {
            debug!(task_id = %task_id, %status, "No workspace repository access; will try the tar");
            return None;
        }
    };

    let started = Instant::now();
    let restored = tokio::task::spawn_blocking({
        let workspace_dir = workspace_dir.clone();
        move || restore(&access, &workspace_dir)
    })
    .await;
    let elapsed = started.elapsed().as_millis() as u64;

    match restored {
        Ok(Ok(Some(bytes))) => {
            info!(task_id = %task_id, bytes, dir = %workspace_dir.display(), "Restored repos from the workspace repository (skipping clone)");
            state
                .backend_client
                .report_workspace_sync(&task_id, "download", FORMAT, bytes, elapsed, None)
                .await;
            // A snapshot that turned out not to carry the primary repo is not
            // one a clone can be skipped after.
            if primary_repo_is_usable(&repo_dir).await {
                Some(RestoreDecision::RestoredSnapshot)
            } else {
                debug!(task_id = %task_id, "Restored tree has no usable primary repo; will clone");
                None
            }
        }
        Ok(Ok(None)) => {
            debug!(task_id = %task_id, "No stored workspace snapshot; will try the tar");
            None
        }
        Ok(Err(error)) => {
            warn!(task_id = %task_id, %error, "Restoring the workspace repository failed (non-fatal); clearing partial state");
            // A snapshot carries the whole workspace — every repo, `uploads/`,
            // loose directories — so a half-finished restore leaves partial
            // files across all of it, not only under `repo_dir`.
            clear_dir_contents(&workspace_dir).await;
            state
                .backend_client
                .report_workspace_sync(&task_id, "download", FORMAT, 0, elapsed, Some(&error))
                .await;
            None
        }
        Err(error) => {
            let error = format!("restore panicked: {error}");
            warn!(task_id = %task_id, %error, "Workspace restore panicked (non-fatal); clearing partial state");
            clear_dir_contents(&workspace_dir).await;
            state
                .backend_client
                .report_workspace_sync(&task_id, "download", FORMAT, 0, elapsed, Some(&error))
                .await;
            None
        }
    }
}

/// Restore the latest snapshot onto `workspace_dir`, or `Ok(None)` when the
/// repository holds none. Returns the bytes written.
fn restore(access: &Access, workspace_dir: &Path) -> Result<Option<u64>, String> {
    let backends = RepositoryBackends::new(Arc::new(access.backend.clone()), None);
    let credentials = RepoCredentials::password(&access.password);
    let repo = Repository::new(&options(), &backends)
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

    let node = match repo.node_from_snapshot_path(&format!("latest:{SNAPSHOT_ROOT}"), |_| true) {
        Ok(node) => node,
        // A repository with a config but no snapshot is a sandbox that created
        // it and was stopped before its first sync.
        Err(error) => {
            debug!(%error, "Workspace repository holds no snapshot yet");
            return Ok(None);
        }
    };

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
    Ok(Some(bytes))
}

/// Drop all but the last [`KEEP_SNAPSHOTS`] snapshots and reclaim their packs.
///
/// Pre-stop only, never per turn: a prune rewrites the index, which is work a
/// turn boundary should not pay for. Best-effort — a repository that keeps a few
/// snapshots too many still restores.
pub(super) async fn forget_and_prune(config: &Config, backend_client: &dyn BackendClient) {
    let task_id = config.task_id.clone();
    let Ok(access) = access(&task_id, backend_client).await else {
        return;
    };
    match tokio::task::spawn_blocking(move || prune(&access)).await {
        Ok(Ok(forgotten)) => {
            debug!(task_id = %task_id, forgotten, "Pruned the workspace repository");
        }
        Ok(Err(error)) => {
            warn!(task_id = %task_id, %error, "Pruning the workspace repository failed (non-fatal)");
        }
        Err(error) => {
            warn!(task_id = %task_id, %error, "Workspace prune panicked (non-fatal)");
        }
    }
}

fn prune(access: &Access) -> Result<usize, String> {
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
    // them 23 h later — and a stopping task has no second run. Measured: with
    // the default, 42 packs / 1.35 GB stayed; with this, 12 packs went and
    // `check` returned to zero findings.
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

    /// The whole point of the format: what comes back is what went in.
    ///
    /// Ignored, because it needs an endpoint. Start one and run it with
    ///
    /// ```bash
    /// docker run -d --name minio-restic -p 19000:9000 \
    ///   -e MINIO_ROOT_USER=spikeadmin -e MINIO_ROOT_PASSWORD=spikeadmin123 \
    ///   minio/minio:latest server /data
    /// cargo test -p moatless-sandbox --lib restic -- --ignored --nocapture
    /// ```
    ///
    /// This is the regression test for the file set. `git_ignore` cannot
    /// produce it — that flag is `.gitignore`-*file* semantics and not the git
    /// index, so a `git add -f`'d file is dropped and the restored repo reports
    /// it deleted. The tree below contains exactly that case.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "needs an S3 endpoint at 127.0.0.1:19000"]
    async fn a_snapshot_restores_the_file_set_byte_for_byte() {
        use std::collections::BTreeMap;
        use std::os::unix::fs::PermissionsExt;

        let source = scratch("source");
        let restored = scratch("restored");
        build_fixture(&source);

        let access = minio_access(&format!("fidelity/{}", std::process::id())).await;
        let outcome = tokio::task::spawn_blocking({
            let source = source.clone();
            let restored = restored.clone();
            move || {
                let added = snapshot(&access, &source)?;
                let bytes = restore(&access, &restored)?;
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
            .map(|file| (file.rel, file.meta))
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
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
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

    /// A repository on the local `MinIO`, addressed by a prefix of its own so
    /// repeated runs do not share state.
    async fn minio_access(prefix: &str) -> Access {
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
        Access {
            backend: ResticS3Backend::new(client, "moatless-restic-test", prefix),
            password: "fidelity".to_string(),
        }
    }
}
