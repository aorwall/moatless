//! Round-trips a restic repository through [`ResticS3Backend`] against a real
//! S3 server.
//!
//! Ignored by default: it needs an endpoint. Start one and run it with
//!
//! ```bash
//! docker run -d --name minio-restic -p 19000:9000 \
//!   -e MINIO_ROOT_USER=spikeadmin -e MINIO_ROOT_PASSWORD=spikeadmin123 \
//!   minio/minio:latest server /data
//! cargo test -p moatless-s3 --features restic-backend --test restic_backend_minio \
//!   -- --ignored --nocapture
//! ```
//!
//! The unit tests cover the key layout against a fake endpoint. What only a
//! real server can show is that `rustic_core` drives this backend through a
//! whole init → backup → read → restore cycle, and that blocking on the
//! backend's own runtime from inside `spawn_blocking` does not deadlock.
#![cfg(feature = "restic-backend")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use aws_sdk_s3::config::{Credentials, Region};
use moatless_s3::ResticS3Backend;
use rustic_core::{
    BackupOptions, Credentials as RepoCredentials, LocalDestination, LsOptions, PathList,
    Repository, RepositoryBackends, RepositoryOptions, RestoreOptions, SnapshotOptions,
};

const ENDPOINT: &str = "http://127.0.0.1:19000";
const BUCKET: &str = "moatless-restic-test";

fn client() -> aws_sdk_s3::Client {
    let config = aws_sdk_s3::Config::builder()
        .behavior_version_latest()
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::new(
            "spikeadmin",
            "spikeadmin123",
            None,
            None,
            "test",
        ))
        .endpoint_url(ENDPOINT)
        .force_path_style(true)
        .build();
    aws_sdk_s3::Client::from_conf(config)
}

/// Create the bucket, ignoring the "it is already there" answer.
async fn ensure_bucket(client: &aws_sdk_s3::Client) {
    _ = client.create_bucket().bucket(BUCKET).send().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs an S3 endpoint at 127.0.0.1:19000"]
async fn a_repository_round_trips_through_the_backend() {
    ensure_bucket(&client()).await;

    let source = tempdir();
    std::fs::create_dir_all(source.join("nested")).unwrap();
    std::fs::write(source.join("top.txt"), b"top level").unwrap();
    std::fs::write(source.join("nested/deep.txt"), vec![b'x'; 400_000]).unwrap();

    let prefix = format!("workspaces/{}", std::process::id());
    let cache = tempdir();
    let restored = tempdir();

    // The production shape: every `rustic_core` call inside one `spawn_blocking`,
    // with the backend blocking on its own runtime underneath.
    let outcome = tokio::task::spawn_blocking(move || {
        let backend = ResticS3Backend::new(client(), BUCKET, &prefix);
        let backends = RepositoryBackends::new(Arc::new(backend), None);
        let options = RepositoryOptions::default().cache_dir(cache.clone());
        let password = RepoCredentials::password("round-trip");

        _ = Repository::new(&options, &backends)
            .unwrap()
            .init(&password, &Default::default(), &Default::default())
            .unwrap();

        let repo = Repository::new(&options, &backends)
            .unwrap()
            .open(&password)
            .unwrap()
            .to_indexed_ids()
            .unwrap();

        let mut backup = BackupOptions::default();
        backup.as_path = Some("/workspace".into());
        let paths = PathList::from_string(&source.to_string_lossy()).unwrap();
        let snapshot = SnapshotOptions::default()
            .label(Some("workspace".to_string()))
            .to_snapshot()
            .unwrap();
        let first = repo.backup(&backup, &paths, snapshot).unwrap();

        // A second snapshot of an unchanged tree must find its parent and add
        // nothing, which is what makes the format incremental. The parent is
        // resolved against the index the repository was opened with, so the
        // handle is reopened rather than reused: a handle opened before the
        // first snapshot cannot see the tree that snapshot wrote, and re-reads
        // every file as new.
        let repo = Repository::new(&options, &backends)
            .unwrap()
            .open(&password)
            .unwrap()
            .to_indexed_ids()
            .unwrap();
        let snapshot = SnapshotOptions::default()
            .label(Some("workspace".to_string()))
            .to_snapshot()
            .unwrap();
        let second = repo.backup(&backup, &paths, snapshot).unwrap();
        let added = second.summary.as_ref().unwrap().data_added;
        let unmodified = second.summary.as_ref().unwrap().files_unmodified;

        let repo = Repository::new(&options, &backends)
            .unwrap()
            .open(&password)
            .unwrap()
            .to_indexed()
            .unwrap();

        // One file, read as a range out of the middle.
        let node = repo
            .node_from_snapshot_path("latest:/workspace/nested/deep.txt", |_| true)
            .unwrap();
        let file = repo.open_file(&node).unwrap();
        let middle = repo.read_file_at(&file, 200_000, 16).unwrap();

        let destination =
            LocalDestination::new(&format!("{}/", restored.display()), true, false).unwrap();
        let tree = repo
            .node_from_snapshot_path("latest:/workspace", |_| true)
            .unwrap();
        let streamer = repo.ls(&tree, &LsOptions::default()).unwrap();
        let restore = RestoreOptions::default();
        let plan = repo
            .prepare_restore(&restore, streamer.clone(), &destination, false)
            .unwrap();
        repo.restore(plan, &restore, streamer, &destination)
            .unwrap();

        (
            first.id != second.id,
            added,
            unmodified,
            middle.to_vec(),
            restored,
        )
    })
    .await
    .unwrap();

    let (distinct, added, unmodified, middle, restored) = outcome;
    assert!(distinct, "each backup writes its own snapshot");
    assert_eq!(added, 0, "an unchanged tree adds no data to the repository");
    assert_eq!(unmodified, 2, "both files are found in the parent snapshot");
    assert_eq!(middle, vec![b'x'; 16], "a range read lands at the offset");
    assert_eq!(
        std::fs::read_to_string(restored.join("top.txt")).unwrap(),
        "top level"
    );
    assert_eq!(
        std::fs::metadata(restored.join("nested/deep.txt"))
            .unwrap()
            .len(),
        400_000
    );
}

/// Left on disk when the test ends: a failure is worth inspecting, and the
/// paths are process-scoped, so a repeated local loop leaves one tree per run
/// under the system temp directory to clear by hand.
fn tempdir() -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "moatless-restic-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}
