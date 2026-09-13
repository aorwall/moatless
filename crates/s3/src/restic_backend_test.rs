use rustic_core::{BytesList, FileType, Id, ReadBackend, WriteBackend};

use crate::ResticS3Backend;
use crate::objects_test::{endpoint, response};

/// A repository id is 32 bytes, so its name on the wire is 64 hex characters.
const ID: &str = "ab12000000000000000000000000000000000000000000000000000000000000";

/// Runs the fake endpoint on a separate runtime, since the backend's own panics
/// if the calling thread already drives one.
///
/// The runtime outlives the test, mirroring the `spawn_blocking` arrangement
/// production uses.
fn harness(exchanges: Vec<(&'static str, String)>) -> (tokio::runtime::Runtime, ResticS3Backend) {
    let server = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let client = server.block_on(async { endpoint(exchanges).await.0 });
    let backend = ResticS3Backend::new(client, "bucket", "workspaces/task-7/");
    (server, backend)
}

#[test]
fn keys_follow_the_restic_layout() {
    let id: Id = ID.parse().unwrap();
    let (_server, backend) = harness(vec![
        (
            "head /bucket/workspaces/task-7/config",
            "HTTP/1.1 200 OK\r\nContent-Length: 155\r\nConnection: close\r\n\r\n".to_string(),
        ),
        (
            "get /bucket/workspaces/task-7/data/ab/",
            response("200 OK", "pack-body"),
        ),
        (
            "get /bucket/workspaces/task-7/snapshots/",
            response("200 OK", "snapshot-body"),
        ),
        (
            "put /bucket/workspaces/task-7/index/",
            response("200 OK", ""),
        ),
        (
            "delete /bucket/workspaces/task-7/keys/",
            response("204 No Content", ""),
        ),
    ]);

    // `config` is one key with the all-zero id, not a directory of ids.
    assert_eq!(
        backend.list_with_size(FileType::Config).unwrap(),
        vec![(Id::default(), 155)]
    );
    // A pack nests under the first two hex characters of its id.
    assert_eq!(
        backend.read_full(FileType::Pack, &id).unwrap(),
        b"pack-body".as_slice()
    );
    assert_eq!(
        backend.read_full(FileType::Snapshot, &id).unwrap(),
        b"snapshot-body".as_slice()
    );
    backend
        .write_bytes(
            FileType::Index,
            &id,
            false,
            BytesList::from(b"idx".to_vec()),
        )
        .unwrap();
    backend.remove(FileType::Key, &id, false).unwrap();
    assert_eq!(
        backend.location(),
        "s3:bucket/workspaces/task-7",
        "a trailing slash on the prefix is trimmed, not repeated into every key"
    );
    assert_eq!(
        backend.warmup_path(FileType::Pack, &id),
        format!("workspaces/task-7/data/ab/{ID}")
    );
}

#[test]
fn an_absent_config_means_no_repository_rather_than_a_failure() {
    let (_server, backend) = harness(vec![(
        "head /bucket/workspaces/task-7/config",
        response("404 Not Found", ""),
    )]);
    assert!(backend.list_with_size(FileType::Config).unwrap().is_empty());
}

#[test]
fn a_listing_keeps_ids_and_drops_everything_else() {
    let body = format!(
        "<ListBucketResult><IsTruncated>false</IsTruncated>\
         <Contents><Key>workspaces/task-7/snapshots/{ID}</Key><Size>41</Size></Contents>\
         <Contents><Key>workspaces/task-7/snapshots/not-an-id</Key><Size>9</Size></Contents>\
         </ListBucketResult>"
    );
    let (_server, backend) = harness(vec![("get /bucket/?", response("200 OK", &body))]);
    assert_eq!(
        backend.list_with_size(FileType::Snapshot).unwrap(),
        vec![(ID.parse::<Id>().unwrap(), 41)]
    );
}

/// A workspace of any size crosses the 1,000-key page limit, so a listing that
/// stops at the first page tells `rustic_core` the packs beyond it were never
/// written.
#[test]
fn a_listing_follows_every_page() {
    let second: &str = "cd34000000000000000000000000000000000000000000000000000000000000";
    let page_one = format!(
        "<ListBucketResult><IsTruncated>true</IsTruncated>\
         <NextContinuationToken>page-2</NextContinuationToken>\
         <Contents><Key>workspaces/task-7/snapshots/{ID}</Key><Size>41</Size></Contents>\
         </ListBucketResult>"
    );
    let page_two = format!(
        "<ListBucketResult><IsTruncated>false</IsTruncated>\
         <Contents><Key>workspaces/task-7/snapshots/{second}</Key><Size>62</Size></Contents>\
         </ListBucketResult>"
    );
    let (_server, backend) = harness(vec![
        ("get /bucket/?", response("200 OK", &page_one)),
        ("continuation-token=page-2", response("200 OK", &page_two)),
    ]);
    assert_eq!(
        backend.list_with_size(FileType::Snapshot).unwrap(),
        vec![
            (ID.parse::<Id>().unwrap(), 41),
            (second.parse::<Id>().unwrap(), 62)
        ],
        "the second page must be fetched and its ids kept"
    );
}

/// The continuation token is the loop's only bound once `IsTruncated` is out of
/// the condition, so a store that repeats one has to end the walk. Without the
/// guard this exhausts the harness's exchanges and hangs.
#[test]
fn a_listing_stops_when_the_continuation_token_does_not_advance() {
    let page = format!(
        "<ListBucketResult><IsTruncated>true</IsTruncated>\
         <NextContinuationToken>stuck</NextContinuationToken>\
         <Contents><Key>workspaces/task-7/snapshots/{ID}</Key><Size>41</Size></Contents>\
         </ListBucketResult>"
    );
    let (_server, backend) = harness(vec![
        ("get /bucket/?", response("200 OK", &page)),
        ("continuation-token=stuck", response("200 OK", &page)),
    ]);
    assert_eq!(
        backend.list_with_size(FileType::Snapshot).unwrap().len(),
        2,
        "both pages are kept, and the repeated token ends the walk"
    );
}

#[test]
fn a_partial_read_asks_for_an_inclusive_byte_range() {
    let id: Id = ID.parse().unwrap();
    let (_server, backend) = harness(vec![(
        "range: bytes=100-355",
        response("206 Partial Content", "blob"),
    )]);
    assert_eq!(
        backend
            .read_partial(FileType::Pack, &id, false, 100, 256)
            .unwrap(),
        b"blob".as_slice()
    );
}
