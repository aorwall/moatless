//! Tests needing an S3 endpoint are `#[ignore]`d. Start one and run them with:
//!
//! ```bash
//! docker run -d --name minio-restic -p 19000:9000 \
//!   -e MINIO_ROOT_USER=spikeadmin -e MINIO_ROOT_PASSWORD=spikeadmin123 \
//!   minio/minio:latest server /data
//! cargo test -p moatless-sandbox --lib restic -- --ignored --nocapture
//! ```

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

/// The glob file must not land in the tree it describes, or it would appear
/// in the next snapshot and change the file set it defines.
#[test]
fn the_glob_file_and_the_cache_stay_out_of_the_workspace() {
    for (path, temp) in [
        (glob_file(), std::env::temp_dir()),
        (cache_dir(), PathBuf::from("/tmp")),
    ] {
        assert!(
            path.starts_with(temp),
            "{} is not under the temp dir",
            path.display()
        );
        assert!(
            !path.starts_with("/opt/moatless/workspace"),
            "{}",
            path.display()
        );
    }
}

/// A file created after the baseline is in neither [`glob_file`] nor the
/// snapshot, so `snapshot` takes the baseline before `write_glob_file` freezes
/// that set: a later one would match on both sides and certify a tree missing
/// the file.
#[test]
fn a_file_that_appears_after_the_baseline_is_not_certified() {
    let dir = scratch("baseline");
    std::fs::write(dir.join("a.txt"), "one").expect("file");

    let check = FinalTreeCheck::start(
        &dir,
        Capture::ShutdownFlush {
            writers_quiet: true,
        },
    );
    assert!(check.holds(&dir), "nothing has touched the tree");

    std::fs::write(dir.join("b.txt"), "two").expect("file");
    assert!(!check.holds(&dir));
}

/// A periodic capture stores the tree while the agent is free to write it, so
/// it can never earn the tag whatever the tree does during the snapshot.
#[test]
fn a_periodic_capture_never_certifies_a_final_tree() {
    let dir = scratch("periodic");
    std::fs::write(dir.join("a.txt"), "one").expect("file");

    for capture in [
        Capture::Periodic,
        Capture::ShutdownFlush {
            writers_quiet: false,
        },
    ] {
        assert!(!FinalTreeCheck::start(&dir, capture).holds(&dir));
    }
}

/// The walk never enters a directory git ignores whole, so a `target/` costs
/// the snapshot nothing, and still reaches a tracked file inside one.
#[tokio::test]
async fn the_walk_skips_an_ignored_directory_but_not_a_tracked_file_in_it() {
    use rustic_core::{LocalSource, ReadSource};

    let _snapshotting = SNAPSHOTTING.lock().await;
    let root = scratch("ignored-dirs");
    build_fixture(&root);
    let repo = root.join("repo");
    std::fs::create_dir_all(repo.join("vendor/deep")).expect("vendor");
    std::fs::write(repo.join("vendor/kept.rs"), "// kept").expect("kept");
    std::fs::write(repo.join("vendor/deep/noise.rs"), "// noise").expect("noise");
    std::fs::write(repo.join(".gitignore"), "*.log\ntarget/\nvendor/\n").expect("gitignore");
    git(&repo, &["add", "-f", "vendor/kept.rs"]);

    write_glob_file(&root).expect("glob file");
    let backup = backup_options();
    let walked: Vec<String> = LocalSource::new(
        backup.ignore_save_opts,
        &backup.excludes,
        &backup.ignore_filter_opts,
        &[&root],
    )
    .expect("source")
    .entries()
    .map(|entry| super::super::sync::rel_from(&root, &entry.expect("entry").path))
    .collect();

    assert!(
        walked.contains(&"repo/vendor/kept.rs".to_string()),
        "{walked:?}"
    );
    assert!(
        walked.contains(&"repo/src/main.rs".to_string()),
        "{walked:?}"
    );
    for pruned in ["repo/target", "repo/vendor/deep"] {
        assert!(
            !walked.iter().any(|path| path.starts_with(pruned)),
            "{pruned}: {walked:?}"
        );
    }
    _ = std::fs::remove_dir_all(&root);
}

// =========================================================================
// Fidelity against a real S3 server
// =========================================================================

/// Every test that calls [`snapshot`] takes this for its whole body.
///
/// [`glob_file`] is one fixed path, so overlapping snapshots in one test
/// process let the second one's file set decide the first one's, and the
/// failure reads as a file the restore lost.
static SNAPSHOTTING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

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
            let added = snapshot(&access, &source, Capture::Periodic, &Phases::start())?;
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
        .files
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

/// The tag is only worth anything if it survives the round trip and
/// stays off every other snapshot, since a restore reads it as proof.
///
/// Runs against the endpoint
/// [`a_snapshot_restores_the_file_set_byte_for_byte`] documents.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs an S3 endpoint at 127.0.0.1:19000"]
async fn only_the_shutdown_flush_tags_its_snapshot_and_the_restore_reads_it_back() {
    let _snapshotting = SNAPSHOTTING.lock().await;

    let source = scratch("tagged-source");
    let after_periodic = scratch("tagged-periodic");
    let after_flush = scratch("tagged-flush");
    build_fixture(&source);

    let access = minio_access(&format!("tagged/{}", std::process::id())).await;
    let (periodic, flushed) = tokio::task::spawn_blocking({
        let source = source.clone();
        let after_periodic = after_periodic.clone();
        let after_flush = after_flush.clone();
        move || {
            _ = snapshot(&access, &source, Capture::Periodic, &Phases::start())?;
            let periodic = restore(&access, &after_periodic, UnreadableSnapshot::Refuse)?
                .expect("the periodic snapshot restores");
            // The tree has to move, or the second snapshot is the first one.
            std::fs::write(source.join("repo/src/main.rs"), "fn main() { todo!() }")
                .expect("a later edit");
            _ = snapshot(
                &access,
                &source,
                Capture::ShutdownFlush {
                    writers_quiet: true,
                },
                &Phases::start(),
            )?;
            let flushed = restore(&access, &after_flush, UnreadableSnapshot::Refuse)?
                .expect("the shutdown snapshot restores");
            Ok::<_, String>((periodic, flushed))
        }
    })
    .await
    .expect("the snapshot task ran")
    .expect("the snapshots and restores succeeded");

    assert!(!periodic.final_flush, "a periodic snapshot carries no tag");
    assert!(
        flushed.final_flush,
        "the restore must read the shutdown flush's tag back"
    );
    // The tag rides on the newest snapshot, so the restore is that tree.
    assert_eq!(
        std::fs::read_to_string(after_flush.join("repo/src/main.rs")).ok(),
        Some("fn main() { todo!() }".to_string())
    );

    for dir in [&source, &after_periodic, &after_flush] {
        _ = std::fs::remove_dir_all(dir);
    }
}

/// A fork never reads the source task's tag as evidence about its own pod.
///
/// Covers the empty-own-repository fallback against a tagged source. Endpoint
/// as [`a_snapshot_restores_the_file_set_byte_for_byte`].
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs an S3 endpoint at 127.0.0.1:19000"]
async fn a_fork_never_inherits_the_source_s_shutdown_tag() {
    let _snapshotting = SNAPSHOTTING.lock().await;

    let run = std::process::id();
    let source_tree = scratch("inherit-source");
    let restored = scratch("inherit-restored");
    build_fixture(&source_tree);

    let source = minio_access(&format!("inherit/{run}/source")).await;
    let fork = Access {
        own: minio_access(&format!("inherit/{run}/own")).await,
        source: Some(source.clone()),
    };

    let restored_snapshot = tokio::task::spawn_blocking({
        let source_tree = source_tree.clone();
        let restored = restored.clone();
        move || {
            // Tagged, so the source looks exactly like a cleanly stopped pod.
            _ = snapshot(
                &source,
                &source_tree,
                Capture::ShutdownFlush {
                    writers_quiet: true,
                },
                &Phases::start(),
            )?;
            restore_own_or_source(&fork, &restored, UnreadableSnapshot::Refuse)
        }
    })
    .await
    .expect("the restore task ran")
    .expect("the fork restored")
    .expect("the source holds a snapshot to fall back to");

    assert!(
        !restored_snapshot.final_flush,
        "the source's tag must not certify the fork's own interrupted pod"
    );

    for dir in [&source_tree, &restored] {
        _ = std::fs::remove_dir_all(dir);
    }
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
    std::os::unix::fs::symlink("plan.md", root.join("notes/current.md")).expect("note link");
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
            snapshot(&source, &source_tree, Capture::Periodic, &Phases::start())
                .expect("the source snapshotted");
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
    let client = S3Storage::at_endpoint(
        "http://127.0.0.1:19000",
        "us-east-1",
        S3Credentials {
            access_key_id: "spikeadmin".into(),
            secret_access_key: "spikeadmin123".into(),
            session_token: None,
        },
    );
    client.ensure_bucket("moatless-restic-test").await;
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
        _ = self.refusals.lock().expect("map lock").insert(id, count);
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
        objects
            .get(&(tpe.to_string(), *id))
            .cloned()
            .ok_or_else(|| {
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
    map_snapshot_as(backend, dir, Capture::Periodic)
}

fn map_snapshot_as(backend: &MapBackend, dir: &Path, capture: Capture) -> rustic_core::Id {
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
    let written = repo.backup(&backup, &paths, snapshot).expect("backup");
    if !capture.may_certify_final_tree() {
        return *written.id;
    }
    // Through the production tagging path, so these tests pin the one that
    // runs: the tag replaces the snapshot rather than being written with it.
    certify_final_tree(&repo, &written);
    *repo
        .get_all_snapshots()
        .expect("snapshots")
        .iter()
        .find(|snapshot| snapshot.tags.contains(SHUTDOWN_TAG))
        .expect("the tagged snapshot")
        .id
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

/// A skipped generation may be the newer one, so the tag on what is left
/// describes a tree the pod moved past.
///
/// The restore still happens — an older tree beats none — but it must come
/// back with the dated note rather than the no-audit one.
#[test]
fn a_tagged_snapshot_beside_an_unreadable_one_is_not_final() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let source = tmp.path().join("source");
    let into = tmp.path().join("into");
    let backend = map_repo();

    write_tree(&source, "first");
    _ = map_snapshot_as(
        &backend,
        &source,
        Capture::ShutdownFlush {
            writers_quiet: true,
        },
    );
    write_tree(&source, "second");
    backend.make_unreadable(map_snapshot(&backend, &source));

    let restored = map_restore(&backend, &into, UnreadableSnapshot::PassOver)
        .expect("the tagged generation still reads")
        .expect("a repository holding two snapshots is not an absence");

    assert_eq!(
        std::fs::read_to_string(into.join("a.txt")).expect("restored file"),
        "first",
        "the tagged generation is the one left to restore"
    );
    assert!(restored.skipped_unreadable);
    assert!(
        !restored.final_flush,
        "a newer generation went unread, so the tag certifies nothing"
    );
}

/// The same tag with nothing skipped is what the restore may act on, so the
/// clearing above is the skip and not the tag being unreadable in general.
#[test]
fn a_tagged_snapshot_with_nothing_skipped_is_final() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let source = tmp.path().join("source");
    let into = tmp.path().join("into");
    let backend = map_repo();

    write_tree(&source, "first");
    _ = map_snapshot_as(
        &backend,
        &source,
        Capture::ShutdownFlush {
            writers_quiet: true,
        },
    );

    let restored = map_restore(&backend, &into, UnreadableSnapshot::PassOver)
        .expect("the tagged generation reads")
        .expect("a repository holding a snapshot is not an absence");

    assert!(!restored.skipped_unreadable);
    assert!(restored.final_flush);
}

/// Tagging replaces a snapshot, so the untagged original must not survive it:
/// a twin of the same tree is a second generation the restore can land on.
#[test]
fn certifying_a_tree_leaves_one_snapshot_carrying_the_tag() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let source = tmp.path().join("source");
    let backend = map_repo();

    write_tree(&source, "first");
    _ = map_snapshot_as(
        &backend,
        &source,
        Capture::ShutdownFlush {
            writers_quiet: true,
        },
    );

    let repo = Repository::new(&map_options(), &map_backends(&backend))
        .expect("repository")
        .open(&RepoCredentials::password(MAP_PASSWORD))
        .expect("open");
    let snapshots = repo.get_all_snapshots().expect("snapshots");
    assert_eq!(snapshots.len(), 1, "the untagged copy was left behind");
    assert!(snapshots[0].tags.contains(SHUTDOWN_TAG));
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

/// Back `dir` up the way [`snapshot`] does, stamped with `host` the way a pod's
/// own hostname stamps it.
fn map_backup_from_pod(backend: &MapBackend, dir: &Path, host: &str) -> SnapshotFile {
    let repo = Repository::new(&map_options(), &map_backends(backend))
        .expect("repository")
        .open(&RepoCredentials::password(MAP_PASSWORD))
        .expect("open")
        .to_indexed_ids()
        .expect("index");
    let mut backup = backup_options();
    // The shared glob file belongs to whichever test wrote it last.
    backup.excludes.glob_files.clear();
    let paths = PathList::from_string(&dir.to_string_lossy()).expect("paths");
    let snapshot = SnapshotOptions::default()
        .label(Some(SNAPSHOT_LABEL.to_string()))
        .host(Some(host.to_string()))
        .to_snapshot()
        .expect("metadata");
    repo.backup(&backup, &paths, snapshot).expect("backup")
}

/// A pod restores a tree another pod stored, and its first snapshot of that
/// tree must reuse the stored one: re-reading every restored file is what ran
/// a 4.7 GiB shutdown flush out of its budget.
#[test]
fn the_first_snapshot_after_a_restore_reads_no_unchanged_file() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let source = tmp.path().join("source");
    let into = tmp.path().join("into");
    let backend = map_repo();

    std::fs::create_dir_all(&source).expect("mkdir");
    for name in ["a.txt", "b.txt", "c.txt"] {
        std::fs::write(source.join(name), name.repeat(1000)).expect("write");
    }
    _ = map_backup_from_pod(&backend, &source, "pod-a");
    _ = map_restore(&backend, &into, UnreadableSnapshot::Refuse)
        .expect("restore")
        .expect("a snapshot");

    let written = map_backup_from_pod(&backend, &into, "pod-b");

    assert!(
        !written.get_parents().is_empty(),
        "the previous pod's snapshot was not picked as the parent"
    );
    let summary = written.summary.expect("summary");
    assert_eq!(
        (
            summary.files_new,
            summary.files_changed,
            summary.files_unmodified
        ),
        (0, 0, 3),
        "restored files were read as new or changed"
    );
}
