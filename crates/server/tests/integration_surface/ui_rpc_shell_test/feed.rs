//! Driving a watching client: opening a feed, making a change, and taking what
//! it delivered.

use super::contract::{assert_decodable_item, assert_decodable_upsert};
use super::harness::{defaults, reads};
use chrono::Utc;
use moatless_core::change_feed::listing::EnvironmentIdentity;
use moatless_core::change_feed::listing::record_change;
use moatless_t3code::OrchestrationProtocol;
use moatless_t3code::ShellFeed;
use moatless_t3code::shell_snapshot;
use moatless_t3code::shell_subscription;
use moatless_tasks::task_turn::models::TurnStateEvent;
use moatless_tasks::task_turn::models::classify_turn_transition;
use moatless_tasks::task_turn::test_support as turn_fixtures;
use moatless_types::access_control::ScopeContext;
use serde_json::Value;
use serde_json::json;
use sqlx::PgPool;

/// A client that has read the snapshot and is now watching — exactly the state
/// `subscribeShell` leaves behind, with the feed resuming from the position the
/// snapshot was read at.
pub(super) async fn watching(pool: &PgPool, viewer: &ScopeContext) -> ShellFeed {
    let snapshot = shell_snapshot(&reads(pool), &defaults(), viewer)
        .await
        .unwrap();
    // Seeded with what the snapshot put in front of them, because that is the
    // only set a removal may ever name. A feed told nothing about what its
    // client holds cannot take anything back — see `ShellFeed`.
    let (threads, projects) = (snapshot.thread_ids(), snapshot.project_ids());
    ShellFeed::resuming_from(snapshot.identity, snapshot.sequence).holding(threads, projects)
}

/// One poll, as the subscription makes it.
pub(super) async fn delivered(
    pool: &PgPool,
    feed: &mut ShellFeed,
    viewer: &ScopeContext,
) -> Vec<Value> {
    feed.poll(&reads(pool), &defaults(), viewer).await.unwrap()
}

/// Exactly one delivered row, decoded — the shape every acceptance criterion
/// here is stated in ("its row moves", not "some events arrive").
pub(super) async fn one_row(pool: &PgPool, feed: &mut ShellFeed, viewer: &ScopeContext) -> Value {
    let items = delivered(pool, feed, viewer).await;
    assert_eq!(
        items.len(),
        1,
        "expected exactly one delivered row, got {items:#?}"
    );
    assert_decodable_upsert(&items[0]);
    items[0].clone()
}

/// Start a turn the way a run does: the allocator, on the run's first signal.
pub(super) async fn start_turn(pool: &PgPool, task_id: &str, run_id: &str) -> i32 {
    turn_fixtures::resolve_turn_for_run(pool, task_id, run_id)
        .await
        .unwrap()
}

/// Applies a state-bearing event through the gRPC ingress writer.
///
/// Classified by the same function the ingress itself uses, so a `(type,
/// sub_type)` pair that stops bearing turn state fails here rather than quietly
/// delivering nothing.
pub(super) async fn ingress(
    pool: &PgPool,
    task_id: &str,
    turn_number: i32,
    event_type: &str,
    sub_type: &str,
) {
    let transition = classify_turn_transition(event_type, sub_type)
        .unwrap_or_else(|| panic!("{event_type}.{sub_type} bears no turn state"));
    turn_fixtures::apply_ingress_event(
        pool,
        &TurnStateEvent {
            event_id: format!("ev-{task_id}-{turn_number}-{event_type}-{sub_type}"),
            task_id: task_id.to_string(),
            event_type: event_type.to_string(),
            sub_type: sub_type.to_string(),
            is_error: sub_type == "error",
            agent_profile: Some("claude-code".to_string()),
            turn_number,
            payload: json!({}),
            timestamp: Utc::now().to_rfc3339(),
            transition,
        },
    )
    .await
    .unwrap();
}

/// How many changes were recorded past a position.
///
/// Confirms a test's premise held: a coalescing assertion means nothing with
/// only one change recorded, and a filtering assertion means nothing if the
/// excluded change was never written.
pub(super) async fn recorded_after(pool: &PgPool, after: i64) -> usize {
    reads(pool)
        .listing_changes_after(after, 100)
        .await
        .unwrap()
        .len()
}

/// Reconnect: what `subscribeShell` hands a client that arrives holding
/// `after`, and the feed it carries on with.
pub(super) async fn reconnect(
    pool: &PgPool,
    viewer: &ScopeContext,
    after: Option<i64>,
) -> (Vec<Value>, ShellFeed) {
    let environment = reads(pool).listing_head().await.unwrap().identity;
    reconnect_to(pool, &environment, viewer, after).await
}

/// The same reconnection, for a deployment that started against `environment`
/// and is now reading `pool`.
///
/// The two are the same database except in the restore tests, where only the
/// database underneath changes — the process and its clients don't.
pub(super) async fn reconnect_to(
    pool: &PgPool,
    environment: &EnvironmentIdentity,
    viewer: &ScopeContext,
    after: Option<i64>,
) -> (Vec<Value>, ShellFeed) {
    let subscription = shell_subscription(
        &reads(pool),
        &defaults(),
        viewer,
        environment,
        after,
        OrchestrationProtocol::V1,
    )
    .await
    .unwrap();
    (subscription.values, subscription.feed)
}

/// Advances the deployment's position past the resume window without pretending
/// the changes belong to anything.
///
/// Recorded through the real allocator on a Task id nothing references, so the
/// head moves as it would after days of other people's work — disconnection
/// criterion 2.
pub(super) async fn other_peoples_work(pool: &PgPool, count: i64) {
    let mut tx = pool.begin().await.unwrap();
    for index in 0..count {
        record_change(&mut tx, &format!("t-elsewhere-{index}"))
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
}

/// Exactly one item, decoded. The counterpart of `one_row` for the deliveries
/// that are not conversation rows.
pub(super) async fn one_item(pool: &PgPool, feed: &mut ShellFeed, viewer: &ScopeContext) -> Value {
    let items = delivered(pool, feed, viewer).await;
    assert_eq!(items.len(), 1, "expected exactly one item, got {items:#?}");
    assert_decodable_item(&items[0]);
    items[0].clone()
}

/// The ids a client holds after applying everything sent, folded exactly as
/// `applyShellStreamEvent` does (`shellReducer.ts`); asserting on this says
/// what is on screen.
///
/// `threads` is the sidebar: an archived conversation is cached but filtered
/// out by `archivedAt`, and only the sidebar half is what these tests cover.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Held {
    pub(super) threads: Vec<String>,
    pub(super) projects: Vec<String>,
    pub(super) sequence: i64,
}

impl Held {
    /// What the snapshot a subscription opens with leaves the client holding.
    pub(super) async fn opening(pool: &PgPool, viewer: &ScopeContext) -> Self {
        let snapshot = shell_snapshot(&reads(pool), &defaults(), viewer)
            .await
            .unwrap();
        let mut held = Held {
            sequence: snapshot.sequence,
            ..Held::default()
        };
        held.apply(&json!({ "kind": "snapshot", "snapshot": snapshot.value }));
        held
    }

    pub(super) fn apply(&mut self, item: &Value) {
        assert_decodable_item(item);
        let ids = |rows: &Value| -> Vec<String> {
            rows.as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap().to_string())
                .collect()
        };
        if item["kind"] == "snapshot" {
            // Applied whole and unconditionally, position not compared.
            self.threads = ids(&item["snapshot"]["threads"]);
            self.projects = ids(&item["snapshot"]["projects"]);
            self.sequence = item["snapshot"]["snapshotSequence"].as_i64().unwrap();
            return;
        }
        let sequence = item["sequence"].as_i64().unwrap();
        if sequence <= self.sequence {
            // The client's own rule, and the reason two events may not share a
            // position: the second is discarded without a word.
            return;
        }
        self.sequence = sequence;
        let upsert = |set: &mut Vec<String>, id: &str| {
            if !set.iter().any(|held| held == id) {
                set.push(id.to_string());
            }
        };
        match item["kind"].as_str().unwrap() {
            "thread-upserted" => {
                let id = item["thread"]["id"].as_str().unwrap();
                if item["thread"]["archivedAt"].is_null() {
                    upsert(&mut self.threads, id);
                } else {
                    self.threads.retain(|held| held != id);
                }
            }
            "thread-removed" => self
                .threads
                .retain(|id| id != item["threadId"].as_str().unwrap()),
            "project-upserted" => {
                upsert(&mut self.projects, item["project"]["id"].as_str().unwrap());
            }
            "project-removed" => self
                .projects
                .retain(|id| id != item["projectId"].as_str().unwrap()),
            _ => unreachable!(),
        }
    }

    /// Poll and fold, returning what was delivered.
    pub(super) async fn poll(
        &mut self,
        pool: &PgPool,
        feed: &mut ShellFeed,
        viewer: &ScopeContext,
    ) -> Vec<Value> {
        let items = delivered(pool, feed, viewer).await;
        for item in &items {
            self.apply(item);
        }
        items
    }
}

/// A watching client, and the listing it is holding, together.
pub(super) async fn watcher(pool: &PgPool, viewer: &ScopeContext) -> (ShellFeed, Held) {
    (
        watching(pool, viewer).await,
        Held::opening(pool, viewer).await,
    )
}
