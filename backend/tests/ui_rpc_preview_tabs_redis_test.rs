#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! The preview tab store, against the Redis it is written for.
//!
//! `InMemoryPreviewTabDao` satisfies the same conformance list in the unit
//! tier, and passing it there says nothing about the implementation that runs
//! in production: the Redis one keeps tabs in a hash, revisions in a separate
//! counter, and fans events over pub/sub, so its atomicity, its ordering and
//! its cross-process reach are all properties of Redis rather than of the code
//! under test. This is the only tier with a Redis to point at.
//!
//! Two separately-connected handles stand in for two replicas. That is the
//! reason the store exists — a client that reconnects to a different pod has to
//! find its tabs, and a tab opened on one pod has to reach the browser
//! subscribed on the other.

use std::time::Duration;

use moatless_vibe_backend::ui_rpc::preview_tabs::{
    conformance, PreviewTabDao, PreviewTabTtl, RedisPreviewTabDao,
};
use redis::AsyncCommands;
use serde_json::json;
use tokio::task::JoinHandle;

/// Long enough that nothing under test expires mid-run, short enough that a
/// killed run leaves no residue anyone has to clean up by hand.
// reason: `duration_suboptimal_units` asks for `from_mins`, which is unstable.
#[allow(clippy::duration_suboptimal_units)]
const TTL: PreviewTabTtl = PreviewTabTtl(Duration::from_secs(120));

/// How long a cross-replica delivery gets before it counts as not happening.
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(10);

/// The Task ids [`conformance`] writes under. They are fixed rather than
/// generated, so this test clears them first and two concurrent runs against
/// one Redis would interfere. Every other key in the database is untouched.
const CONFORMANCE_TASKS: [&str; 2] = ["task", "other"];

fn redis_url() -> Option<String> {
    std::env::var("REDIS_URL").ok().filter(|url| !url.is_empty())
}

async fn clear(url: &str, task_ids: &[&str]) {
    let client = redis::Client::open(url).unwrap();
    let mut conn = client.get_multiplexed_async_connection().await.unwrap();
    for task_id in task_ids {
        let _: () = conn
            .del(&[format!("preview:{task_id}"), format!("preview:{task_id}:rev")])
            .await
            .unwrap();
    }
}

async fn connect(url: &str) -> (RedisPreviewTabDao, JoinHandle<()>) {
    RedisPreviewTabDao::connect(url, TTL)
        .await
        .expect("REDIS_URL is set, so it is expected to be reachable")
}

/// Publish on one handle until the event lands on the other's subscriber.
///
/// Two things make a single publish the wrong assertion. Redis pub/sub reaches
/// whoever is subscribed at that instant and nobody else, and a handle's
/// forwarder subscribes asynchronously after `connect` returns — so an early
/// publish is simply dropped. And the database is shared with whatever else is
/// pointed at it, so the next event to arrive is not necessarily this one.
async fn expect_delivery(
    from: &RedisPreviewTabDao,
    events: &mut tokio::sync::broadcast::Receiver<serde_json::Value>,
    tag: &str,
) {
    let delivered = tokio::time::timeout(DELIVERY_TIMEOUT, async {
        loop {
            from.publish(json!({ "tag": tag, "type": "opened" }))
                .await
                .unwrap();
            let deadline = tokio::time::Instant::now() + Duration::from_millis(250);
            while let Ok(Ok(event)) = tokio::time::timeout_at(deadline, events.recv()).await {
                if event.get("tag").and_then(serde_json::Value::as_str) == Some(tag) {
                    return;
                }
            }
        }
    })
    .await;
    assert!(
        delivered.is_ok(),
        "an event published on one replica never reached the other"
    );
}

/// The whole contract, against the implementation that ships.
#[tokio::test]
async fn the_redis_store_satisfies_the_same_contract_as_the_in_memory_one() {
    let Some(url) = redis_url() else {
        println!("REDIS_URL not set — skipping the Redis preview tab conformance run");
        return;
    };

    clear(&url, &CONFORMANCE_TASKS).await;
    let (store, forwarder) = connect(&url).await;

    conformance::run_every_case(&store).await;

    forwarder.abort();
    clear(&url, &CONFORMANCE_TASKS).await;
}

/// What a second replica sees, which is the reason any of this is in Redis.
#[tokio::test]
async fn two_replicas_share_one_set_of_tabs_and_one_stream_of_events() {
    let Some(url) = redis_url() else {
        println!("REDIS_URL not set — skipping the two-replica preview tab test");
        return;
    };

    let task_id = format!("replica-test-{}", uuid_like());
    clear(&url, &[task_id.as_str()]).await;
    let (left, left_forwarder) = connect(&url).await;
    let (right, right_forwarder) = connect(&url).await;

    // The epoch identifies the store's contents, not the process reading them.
    // A client that reconnects to the other replica has to be told the same
    // one, or it would discard revisions that are still comparable.
    assert_eq!(left.epoch(), right.epoch());

    // A tab opened on one replica is a tab the other one lists.
    let opened = conformance::tab(&task_id, "a", "https://a");
    let revision = left.put(&opened).await.unwrap();
    let seen = right.list(&task_id).await.unwrap();
    assert_eq!(seen.tabs, vec![opened]);
    assert_eq!(seen.revision, revision);

    // Revisions come from the store, not from a handle, so the second replica's
    // write continues the first's sequence rather than restarting it.
    let next = right
        .put(&conformance::tab(&task_id, "b", "https://b"))
        .await
        .unwrap();
    assert!(next > revision, "{next} should follow {revision}");

    // An event published on one replica reaches a browser subscribed on the
    // other. Without this a tab opened from a second window would render and
    // then never update.
    let mut on_the_right = right.subscribe();
    expect_delivery(&left, &mut on_the_right, &task_id).await;

    // Closing reaches across too: the tabs are one set, not two.
    let removal = right.remove(&task_id, None).await.unwrap();
    assert_eq!(removal.tab_ids, vec!["a".to_string(), "b".to_string()]);
    assert_eq!(left.list(&task_id).await.unwrap().tabs, Vec::new());

    left_forwarder.abort();
    right_forwarder.abort();
    clear(&url, &[task_id.as_str()]).await;
}

/// A per-run suffix, so two runs against one Redis do not share a Task id.
///
/// The tabs are keyed by Task and nothing here needs the value to be a real
/// UUID — only that a concurrent run picked a different one.
fn uuid_like() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{nanos:x}")
}
