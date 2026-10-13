#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! The position a watching client resumes from, and the one property it rests
//! on.
//!
//! A client polls `WHERE position > N` and moves its cursor forward, never
//! looking back — so the only failure mode is a record becoming visible *below*
//! a position already passed, silently stalling a conversation.
//!
//! Not observable without a real database and two concurrent transactions: the
//! mechanism is Postgres's row locking and commit visibility.

use crate::common;

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use moatless_core::change_feed::listing::ListingHead;
use moatless_core::change_feed::listing::RESUME_WINDOW;
use moatless_core::change_feed::listing::RETAINED_CHANGES;
use moatless_core::change_feed::listing::Resume;
use moatless_core::change_feed::listing::changes_after;
use moatless_core::change_feed::listing::current_head;
use moatless_core::change_feed::listing::current_position;
use moatless_core::change_feed::listing::prune_changes;
use moatless_core::change_feed::listing::record_change;
use moatless_core::change_feed::listing::resume;
use moatless_tasks::tag::test_support as tag_fixtures;
use moatless_tasks::task::models::TaskPatch;
use moatless_tasks::task::models::TaskStatus;
use moatless_tasks::task::models::Visibility;
use moatless_tasks::task_turn::models::TurnStateEvent;
use moatless_tasks::task_turn::models::TurnStatus;
use moatless_tasks::task_turn::models::TurnTransition;
use moatless_tasks::task_turn::test_support as turn_fixtures;
use moatless_types::access_control::ScopeContext;
use moatless_workspaces::compose;
use moatless_workspaces::workspace::UpdateOutcome;
use moatless_workspaces::workspace::models::WorkspacePatch;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use common::{USER_1, make_task_row, make_workspace_row};

/// Writers and mutations for the contention measurement at the end of the file.
const WRITERS: usize = 32;
const PER_WRITER: usize = 25;

/// A second pool onto the same test database, sized for the concurrency a test
/// drives. `#[sqlx::test]`'s own pool is small on purpose, and queuing on it
/// would measure the pool rather than the counter.
async fn wide_pool(pool: &PgPool, connections: u32) -> PgPool {
    PgPoolOptions::new()
        .max_connections(connections)
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap()
}

/// The positions a client would observe, polling from scratch.
async fn observed(pool: &PgPool) -> Vec<i64> {
    changes_after(pool, 0, 1000)
        .await
        .unwrap()
        .into_iter()
        .map(|change| change.position)
        .collect()
}

// =========================================================================
// The property
// =========================================================================

/// The second writer cannot finish allocating a position until the first
/// commits.
///
/// An allocator that hands out a value without holding it (`bigserial`, a
/// sequence, `MAX(id) + 1`) would let the second finish immediately instead,
/// breaking commit order.
#[sqlx::test]
async fn a_second_writer_cannot_take_a_position_until_the_first_commits(pool: PgPool) {
    let mut first = pool.begin().await.unwrap();
    let first_position = record_change(&mut first, "task-first").await.unwrap();
    assert_eq!(first_position, 1);

    // Nothing is observable yet: the first writer has not committed.
    assert!(observed(&pool).await.is_empty());

    let second_pool = pool.clone();
    let second = tokio::spawn(async move {
        let mut tx = second_pool.begin().await.unwrap();
        let position = record_change(&mut tx, "task-second").await.unwrap();
        tx.commit().await.unwrap();
        position
    });

    // The teeth. A sequence would have answered long before this elapsed.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !second.is_finished(),
        "the second writer took a position while the first still held the counter; \
         positions can now commit out of order and a polling client will skip one"
    );

    // Still nothing observable — neither writer has committed.
    assert!(observed(&pool).await.is_empty());

    first.commit().await.unwrap();
    let second_position = second.await.unwrap();
    assert_eq!(second_position, 2);

    // In order, and complete. Neither position was ever skippable.
    assert_eq!(observed(&pool).await, vec![1, 2]);
}

/// A client polling while fast writers overtake a slow one sees a prefix of the
/// changes, never a hole.
///
/// Deliberately not a fair race: the slow writer holds its transaction open
/// long enough to guarantee the fast ones commit first, so a `nextval`
/// allocator would fail this on the first poll.
#[sqlx::test]
async fn a_polling_client_never_sees_a_hole(pool: PgPool) {
    const FAST_WRITERS: usize = 5;
    const HELD_FOR: Duration = Duration::from_millis(400);

    let writers = wide_pool(&pool, 12).await;
    let reader = pool.clone();

    let polling = tokio::spawn(async move {
        let mut cursor = 0i64;
        let mut seen = Vec::new();
        for _ in 0..180 {
            for change in changes_after(&reader, cursor, 100).await.unwrap() {
                seen.push(change.position);
                cursor = change.position;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        seen
    });

    let slow = tokio::spawn({
        let pool = writers.clone();
        async move {
            let mut tx = pool.begin().await.unwrap();
            record_change(&mut tx, "task-slow").await.unwrap();
            tokio::time::sleep(HELD_FOR).await;
            tx.commit().await.unwrap();
        }
    });

    // Long enough for the slow writer to have taken its position, short enough
    // that it is still holding it.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let mut fast = Vec::new();
    for writer in 0..FAST_WRITERS {
        let pool = writers.clone();
        fast.push(tokio::spawn(async move {
            let mut tx = pool.begin().await.unwrap();
            record_change(&mut tx, &format!("task-fast-{writer}"))
                .await
                .unwrap();
            tx.commit().await.unwrap();
        }));
    }
    for handle in fast {
        handle.await.unwrap();
    }
    slow.await.unwrap();

    let seen = polling.await.unwrap();
    let total = (FAST_WRITERS + 1) as i64;
    // Contiguous from 1, in order: the slow writer's position was never
    // skippable, however far ahead the fast ones got.
    assert_eq!(
        seen,
        (1..=total).collect::<Vec<i64>>(),
        "a polling client observed positions out of order or with a gap"
    );
    assert_eq!(current_position(&pool).await.unwrap(), total);
    assert_eq!(observed(&pool).await.len(), total as usize);
}

/// The negative control: reproduces on a `bigserial` the hazard the counter
/// prevents.
///
/// `bigserial` assigns its value at INSERT but makes it visible at COMMIT, so
/// two writers take 1 and 2, the second commits first, and a reader polling
/// past 0 sees 2 and never 1.
#[sqlx::test]
async fn a_bigserial_position_lets_a_polling_client_skip_a_committed_change(pool: PgPool) {
    sqlx::query(
        "CREATE TABLE naive_changes (position bigserial PRIMARY KEY, task_id text NOT NULL)",
    )
    .execute(&pool)
    .await
    .unwrap();

    async fn naive_poll(pool: &PgPool, after: i64) -> Vec<i64> {
        sqlx::query_scalar(
            "SELECT position FROM naive_changes WHERE position > $1 ORDER BY position",
        )
        .bind(after)
        .fetch_all(pool)
        .await
        .unwrap()
    }

    let mut slow = pool.begin().await.unwrap();
    let slow_position: i64 = sqlx::query_scalar(
        "INSERT INTO naive_changes (task_id) VALUES ('slow') RETURNING position",
    )
    .fetch_one(&mut *slow)
    .await
    .unwrap();
    assert_eq!(slow_position, 1);

    // The second writer takes the next id and commits immediately. It is not
    // blocked, because a sequence hands its value out without holding anything.
    let mut fast = pool.begin().await.unwrap();
    let fast_position: i64 = sqlx::query_scalar(
        "INSERT INTO naive_changes (task_id) VALUES ('fast') RETURNING position",
    )
    .fetch_one(&mut *fast)
    .await
    .unwrap();
    assert_eq!(fast_position, 2);
    fast.commit().await.unwrap();

    // A client polling in this window sees only the later change and moves its
    // cursor past the earlier one.
    let batch = naive_poll(&pool, 0).await;
    assert_eq!(batch, vec![2], "the uncommitted earlier id is invisible");
    let cursor = *batch.last().unwrap();

    slow.commit().await.unwrap();

    // Position 1 is committed and durable, and this client will never see it.
    assert_eq!(
        naive_poll(&pool, cursor).await,
        Vec::<i64>::new(),
        "a bigserial cursor is expected to skip the earlier commit; if this now \
         passes, re-derive whether the counter is still necessary"
    );
    assert_eq!(naive_poll(&pool, 0).await, vec![1, 2]);
}

// =========================================================================
// Atomicity
// =========================================================================

/// A change and its record commit together, or neither commits.
///
/// Written on the caller's transaction, so a rollback takes it with the
/// mutation; a `record_change` handed the pool instead would survive it.
#[sqlx::test]
async fn a_rolled_back_mutation_leaves_no_record_and_no_position(pool: PgPool) {
    let mut tx = pool.begin().await.unwrap();

    sqlx::query(
        "INSERT INTO task_turns (task_id, turn_number, status) VALUES ('task-a', 1, 'active')",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    let position = record_change(&mut tx, "task-a").await.unwrap();
    assert_eq!(position, 1);

    // Inside the transaction both writes exist; outside it, neither does.
    assert!(observed(&pool).await.is_empty());
    assert_eq!(current_position(&pool).await.unwrap(), 0);

    tx.rollback().await.unwrap();

    assert!(observed(&pool).await.is_empty());
    assert_eq!(current_position(&pool).await.unwrap(), 0);
    let turns: i64 = sqlx::query_scalar("SELECT count(*) FROM task_turns")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(turns, 0);
}

#[sqlx::test]
async fn a_committed_mutation_carries_its_record_with_it(pool: PgPool) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO task_turns (task_id, turn_number, status) VALUES ('task-a', 1, 'active')",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    record_change(&mut tx, "task-a").await.unwrap();
    tx.commit().await.unwrap();

    let changes = changes_after(&pool, 0, 10).await.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].task_id, "task-a");
    assert_eq!(changes[0].position, 1);
    assert_eq!(current_position(&pool).await.unwrap(), 1);
}

/// Workspace state is snapshot-owned. Creating, updating, deleting, or
/// tombstoning one must not consume a Task-listing position.
#[sqlx::test]
async fn workspace_mutations_do_not_advance_the_task_listing(pool: PgPool) {
    let workspaces = compose::workspaces(pool.clone());
    let viewer = owner();

    workspaces
        .insert_workspace(&make_workspace_row("ws-manual", USER_1))
        .await
        .unwrap();
    assert_eq!(
        workspaces
            .update(
                "ws-manual",
                &WorkspacePatch {
                    name: Some("Renamed".to_string()),
                    ..WorkspacePatch::default()
                },
                &viewer,
            )
            .await
            .unwrap(),
        UpdateOutcome::Applied
    );
    assert!(workspaces.soft_delete("ws-manual", &viewer).await.unwrap());

    let mut git = make_workspace_row("ws-git", USER_1);
    git.source = moatless_workspace_types::workspace::WorkspaceSource::Git;
    git.source_repository_id = Some("repo-a".to_string());
    git.source_key = Some("api".to_string());
    workspaces.insert_workspace(&git).await.unwrap();
    assert!(
        workspaces
            .soft_delete_git_origin("ws-git", &viewer)
            .await
            .unwrap()
    );

    assert_eq!(current_position(&pool).await.unwrap(), 0);
    assert!(changes_after(&pool, 0, 10).await.unwrap().is_empty());
}

/// Migration 0130 contracts the record to exactly one non-null Task id while
/// leaving the independently-owned position counter in place.
#[sqlx::test]
async fn the_task_only_schema_rejects_null_ids_and_preserves_counter_gaps(pool: PgPool) {
    let columns: Vec<(String, String)> = sqlx::query_as(
        "SELECT column_name, is_nullable FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'task_listing_changes' \
         ORDER BY ordinal_position",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(columns.contains(&("task_id".to_string(), "NO".to_string())));
    assert!(!columns.iter().any(|(name, _)| name == "workspace_id"));

    let null_insert =
        sqlx::query("INSERT INTO task_listing_changes (position, task_id) VALUES (1, NULL)")
            .execute(&pool)
            .await;
    assert!(null_insert.is_err(), "task_id unexpectedly accepted NULL");

    sqlx::query("UPDATE task_listing_position SET position = 7 WHERE id = TRUE")
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    assert_eq!(record_change(&mut tx, "task-after-gap").await.unwrap(), 8);
    tx.commit().await.unwrap();
    assert_eq!(current_position(&pool).await.unwrap(), 8);
    assert_eq!(observed(&pool).await, vec![8]);
}

/// A client that has caught up asks from the position it holds and is told
/// nothing, rather than handed the same change again.
#[sqlx::test]
async fn a_caught_up_client_is_told_nothing(pool: PgPool) {
    let mut tx = pool.begin().await.unwrap();
    record_change(&mut tx, "task-a").await.unwrap();
    tx.commit().await.unwrap();

    let head = current_position(&pool).await.unwrap();
    assert_eq!(head, 1);
    assert!(changes_after(&pool, head, 10).await.unwrap().is_empty());
}

// =========================================================================
// Retention, and the window a client may resume from
// =========================================================================

/// Advances the counter by `count`, recording one change per position.
///
/// One transaction: the point is the counter's value, not concurrency.
async fn record_many(pool: &PgPool, count: i64) {
    let mut tx = pool.begin().await.unwrap();
    for index in 0..count {
        record_change(&mut tx, &format!("task-{}", index % 7))
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
}

async fn retained(pool: &PgPool) -> Vec<i64> {
    sqlx::query_scalar("SELECT position FROM task_listing_changes ORDER BY position")
        .fetch_all(pool)
        .await
        .unwrap()
}

/// Pruning keeps the retention window and deletes below it, without touching
/// the counter.
///
/// The trap: deriving `current_position` from
/// `MAX(task_listing_changes.position)` would hand out a resume point the
/// deployment passed long ago, and pruning is where that would first surface.
#[sqlx::test]
async fn pruning_keeps_the_retention_window_and_leaves_the_counter_alone(pool: PgPool) {
    let head = RETAINED_CHANGES + 12;
    record_many(&pool, head).await;
    assert_eq!(current_position(&pool).await.unwrap(), head);

    let deleted = prune_changes(&pool).await.unwrap();
    assert_eq!(
        deleted, 12,
        "everything at or below head - RETAINED_CHANGES"
    );

    let kept = retained(&pool).await;
    assert_eq!(kept.len(), RETAINED_CHANGES as usize);
    assert_eq!(kept.first().copied(), Some(13));
    assert_eq!(kept.last().copied(), Some(head));

    // The counter did not move, and neither did what a client resumes from.
    assert_eq!(current_position(&pool).await.unwrap(), head);

    // Idempotent, and safe from any replica: a second prune finds nothing.
    assert_eq!(prune_changes(&pool).await.unwrap(), 0);
}

/// Nothing is deleted before the deployment has reached the retention window.
#[sqlx::test]
async fn a_young_deployment_prunes_nothing(pool: PgPool) {
    record_many(&pool, 40).await;
    assert_eq!(prune_changes(&pool).await.unwrap(), 0);
    assert_eq!(retained(&pool).await.len(), 40);
}

/// The invariant: a cursor `resume` admits still has its records by the time
/// the catch-up reads them.
///
/// Reproduces the production order — decide, then prune against a newer head,
/// then catch up — rather than "prune, then check", which has no teeth: it
/// still passes even when `RETAINED_CHANGES = RESUME_WINDOW`.
#[sqlx::test]
async fn a_prune_cannot_delete_a_record_resume_has_already_promised(pool: PgPool) {
    let head = RETAINED_CHANGES + 500;
    record_many(&pool, head).await;

    // 1. A client reconnects and is admitted, against the head as it is now.
    //    Same database throughout, so the identity guard is satisfied and the
    //    retention rule is the only thing deciding.
    let ListingHead { identity, position } = current_head(&pool).await.unwrap();
    let oldest_admitted = head - RESUME_WINDOW;
    assert_eq!(
        resume(&identity, &identity, position, oldest_admitted),
        Resume::From(oldest_admitted),
        "the boundary cursor this test is about is not admitted"
    );
    assert_eq!(
        resume(&identity, &identity, head, oldest_admitted - 1),
        Resume::Rebuild
    );

    // 2. The deployment moves on while the subscription is still being set up.
    record_many(&pool, 1).await;

    // 3. The pruner ticks, against the newer head.
    prune_changes(&pool).await.unwrap();

    // 4. Only now does the catch-up read. It must still see everything after
    //    the cursor it was promised, with nothing missing from the front.
    let replayed: Vec<i64> = changes_after(&pool, oldest_admitted, RESUME_WINDOW + 10)
        .await
        .unwrap()
        .into_iter()
        .map(|change| change.position)
        .collect();
    // Asserted as first-position-then-shape rather than as a 5,000-element
    // vector, so a failure reads as the one-position hole it is.
    assert_eq!(
        replayed.first().copied(),
        Some(oldest_admitted + 1),
        "a prune deleted a record `resume` had already promised; the client is \
         handed silence where that change belonged"
    );
    assert_eq!(replayed.last().copied(), Some(head + 1));
    assert_eq!(replayed.len(), (RESUME_WINDOW + 1) as usize);
}

// =========================================================================
// Lock order
// =========================================================================

/// A writer waiting on any other lock must not be holding the counter.
///
/// Every mutation locks the `tasks` row before recording a change; recording
/// first inverts that order and deadlocks intermittently under load.
#[sqlx::test]
async fn a_writer_blocked_on_a_task_row_is_not_holding_the_counter(pool: PgPool) {
    sqlx::query(
        r#"INSERT INTO tasks (task_id, owner_user_id, created_at, last_message_at, agent_profile,
                              repository_id, status, total_cost_usd)
           VALUES ('task-a', 'user-1', NOW(), NOW(), 'claude_code', 'repo-a', 'open', 0)"#,
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO task_turns (task_id, turn_number, status, started_at) \
         VALUES ('task-a', 1, 'active', NOW())",
    )
    .execute(&pool)
    .await
    .unwrap();

    let wide = wide_pool(&pool, 4).await;

    // Someone renames the conversation and has not committed: they hold the
    // `tasks` row. This is the shape of every change kind ticket 05 adds.
    let mut renaming = wide.begin().await.unwrap();
    sqlx::query("UPDATE tasks SET name = 'renamed' WHERE task_id = 'task-a'")
        .execute(&mut *renaming)
        .await
        .unwrap();

    // Meanwhile the turn ends. The ingress projects the turn, then blocks on
    // the `tasks` row the rename is holding.
    let ingress_pool = wide.clone();
    let ingress = tokio::spawn(async move {
        turn_fixtures::apply_ingress_event(
            &ingress_pool,
            &TurnStateEvent {
                event_id: "ev-1".to_string(),
                task_id: "task-a".to_string(),
                event_type: "agent".to_string(),
                sub_type: "completed".to_string(),
                is_error: false,
                agent_profile: Some("claude-code".to_string()),
                turn_number: 1,
                payload: serde_json::json!({}),
                timestamp: "2026-01-01T00:00:00Z".to_string(),
                transition: TurnTransition::Terminal(TurnStatus::Completed),
            },
        )
        .await
        .unwrap();
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !ingress.is_finished(),
        "the ingress should still be waiting on the `tasks` row the rename holds"
    );

    // The counter is free while it waits. Against the other lock order this
    // times out: the ingress would be holding the counter and blocking on the
    // `tasks` row, which is one half of the cycle.
    let mut unrelated = wide.begin().await.unwrap();
    let position = tokio::time::timeout(
        Duration::from_secs(3),
        record_change(&mut unrelated, "task-b"),
    )
    .await
    .expect(
        "a writer blocked on the `tasks` row was holding the counter; \
         every conversation mutation in the deployment is now queued behind it, \
         and the pair deadlocks as soon as one takes the two locks the other way",
    )
    .unwrap();
    unrelated.commit().await.unwrap();
    assert_eq!(position, 1, "the unrelated writer allocated first");

    renaming.commit().await.unwrap();
    ingress.await.unwrap();

    // And the ingress still recorded its own change, after the rename let go.
    assert_eq!(current_position(&pool).await.unwrap(), 2);
    let changes = changes_after(&pool, 0, 10).await.unwrap();
    assert_eq!(changes[1].task_id, "task-a");
    assert_eq!(changes[1].position, 2);
}

/// One test case for the invariant: no mutation may hold the counter while it
/// waits on another lock.
///
/// A third party holds the row the mutation must take; the mutation is asserted
/// *waiting* on it, then the counter must be taken on a third connection inside
/// the timeout.
struct Mutation {
    /// What a person did, in the words the acceptance criteria use.
    what: &'static str,
    /// Extra committed state this case needs. `$1` is the case's subject id.
    seed: Option<&'static str>,
    /// The row the mutation must take before it may take the counter, locked by
    /// a third party for the length of the check. `$1` is the subject id.
    hold: &'static str,
    /// The mutation itself, through the same DAO production calls.
    run: fn(PgPool, String) -> BoxFuture<'static, ()>,
}

/// A Task already in the deployment, owned by the user the mutations run as.
async fn seed_subject(pool: &PgPool, subject: &str) {
    crate::common::seed_workspace(pool, crate::common::FIXTURE_WORKSPACE, USER_1).await;
    moatless_tasks::task::test_support::seed_task(pool, &make_task_row(subject, USER_1))
        .await
        .unwrap();
}

fn owner() -> ScopeContext {
    ScopeContext::user(USER_1.to_string())
}

/// The `tasks` row, taken outright — the lock every conversation mutation that
/// rewrites the row itself contends for.
const HOLD_TASK_ROW: &str = "SELECT task_id FROM tasks WHERE task_id = $1 FOR UPDATE";

#[sqlx::test]
async fn no_mutation_holds_the_counter_while_it_waits_for_another_lock(pool: PgPool) {
    sqlx::query("INSERT INTO tags (tag_id, name) VALUES ('tag-1', 'urgent')")
        .execute(&pool)
        .await
        .unwrap();

    let cases: &[Mutation] = &[
        Mutation {
            what: "a conversation is created",
            seed: None,
            // Creation writes no existing row, so what it can be made to wait on
            // is the uniqueness of the id it is about to take.
            hold: "INSERT INTO tasks (task_id, owner_user_id, created_at, last_message_at, \
                   agent_profile, repository_id, status, total_cost_usd) \
                   VALUES ($1 || '-new', 'user-x', NOW(), NOW(), 'claude_code', 'repo-a', \
                   'open', 0)",
            run: |pool, subject| {
                Box::pin(async move {
                    let row = make_task_row(&format!("{subject}-new"), USER_1);
                    moatless_tasks::task::test_support::seed_task(&pool, &row)
                        .await
                        .unwrap();
                })
            },
        },
        Mutation {
            // One statement per field, one transaction, one record: the rename,
            // the close, the visibility change and the new expiry are the same
            // shape and are asserted together.
            what: "a conversation is renamed, closed, made private and given a new expiry",
            seed: None,
            hold: HOLD_TASK_ROW,
            run: |pool, subject| {
                Box::pin(async move {
                    let patch = TaskPatch {
                        name: Some("renamed".to_string()),
                        description: Some("described".to_string()),
                        status: Some(TaskStatus::Closed),
                        visibility: Some(Visibility::Public),
                        ttl_minutes: Some(120),
                        ..TaskPatch::default()
                    };
                    moatless_tasks::task::test_support::patch_task(
                        &pool,
                        &subject,
                        &patch,
                        &owner(),
                    )
                    .await
                    .unwrap();
                })
            },
        },
        Mutation {
            what: "a conversation is closed by the guarded status write",
            seed: None,
            hold: HOLD_TASK_ROW,
            run: |pool, subject| {
                Box::pin(async move {
                    moatless_tasks::task::test_support::set_status_if_open(
                        &pool,
                        &subject,
                        TaskStatus::Closed,
                        &owner(),
                    )
                    .await
                    .unwrap();
                })
            },
        },
        Mutation {
            what: "a conversation is deleted",
            seed: None,
            hold: HOLD_TASK_ROW,
            run: |pool, subject| {
                Box::pin(async move {
                    assert!(
                        moatless_tasks::task::test_support::remove_task(&pool, &subject, &owner())
                            .await
                            .unwrap()
                    );
                })
            },
        },
        Mutation {
            what: "a person writes a message",
            seed: None,
            // The row a send contends for is its own: the inbox item's id,
            // which both sides derive from the subject.
            hold: "INSERT INTO task_inbox (id, task_id, text, origin) \
                   VALUES (md5($1 || '-msg')::uuid, $1, '', 'system')",
            run: |pool, subject| {
                Box::pin(async move {
                    let id: uuid::Uuid = sqlx::query_scalar("SELECT md5($1 || '-msg')::uuid")
                        .bind(&subject)
                        .fetch_one(&pool)
                        .await
                        .unwrap();
                    moatless_tasks::message::test_support::queue_message(
                        &pool,
                        moatless_tasks::message::test_support::user_item(&subject, id, "hello"),
                    )
                    .await
                    .unwrap();
                })
            },
        },
        Mutation {
            what: "a tag is added",
            seed: None,
            // `ON CONFLICT DO NOTHING` waits on another transaction's
            // speculative insertion of the same pair, so an uncommitted row is
            // what blocks this one; a committed one would simply be skipped.
            hold: "INSERT INTO task_tags (task_id, tag_id) VALUES ($1, 'tag-1')",
            run: |pool, subject| {
                Box::pin(async move {
                    tag_fixtures::add_task_tags(&pool, &subject, &["tag-1".to_string()])
                        .await
                        .unwrap();
                })
            },
        },
        Mutation {
            what: "the tag set is replaced",
            seed: Some("INSERT INTO task_tags (task_id, tag_id) VALUES ($1, 'tag-1')"),
            hold: "SELECT id FROM task_tags WHERE task_id = $1 FOR UPDATE",
            run: |pool, subject| {
                Box::pin(async move {
                    tag_fixtures::set_task_tags(&pool, &subject, &["tag-1".to_string()])
                        .await
                        .unwrap();
                })
            },
        },
        Mutation {
            what: "someone starts following a conversation",
            seed: None,
            hold: "INSERT INTO task_follows (user_id, task_id) VALUES ('user-x', $1)",
            run: |pool, subject| {
                Box::pin(async move {
                    moatless_tasks::task::test_support::follow_task(&pool, "user-x", &subject)
                        .await
                        .unwrap();
                })
            },
        },
        Mutation {
            what: "someone stops following a conversation",
            // `insert_task` auto-follows the creator, so the row is already
            // there; this is the follow the unfollow below deletes.
            seed: None,
            hold: "SELECT task_id FROM task_follows WHERE task_id = $1 FOR UPDATE",
            run: |pool, subject| {
                Box::pin(async move {
                    moatless_tasks::task::test_support::unfollow_task(&pool, USER_1, &subject)
                        .await
                        .unwrap();
                })
            },
        },
    ];

    // Four per case: the holder, the mutation, the counter probe, and one spare.
    let wide = wide_pool(&pool, 8).await;

    for (index, case) in cases.iter().enumerate() {
        let subject = format!("subject-{index}");
        seed_subject(&pool, &subject).await;
        if let Some(seed) = case.seed {
            sqlx::query(seed)
                .bind(&subject)
                .execute(&pool)
                .await
                .unwrap();
        }

        let mut holder = wide.begin().await.unwrap();
        sqlx::query(case.hold)
            .bind(&subject)
            .execute(&mut *holder)
            .await
            .unwrap();

        let mutation = tokio::spawn((case.run)(wide.clone(), subject.clone()));

        // Without this the probe below would pass against either lock order,
        // because the mutation would already have committed and be holding
        // nothing at all.
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !mutation.is_finished(),
            "`{}` did not wait for the row it must take before the counter, \
             so this case proves nothing about the order it takes them in",
            case.what
        );

        let mut unrelated = wide.begin().await.unwrap();
        tokio::time::timeout(
            Duration::from_secs(3),
            record_change(&mut unrelated, "unrelated"),
        )
        .await
        .unwrap_or_else(|_| {
            panic!(
                "`{}` was holding the deployment's counter while it waited for \
                 another lock; every mutation in the deployment is now queued \
                 behind it, and the pair deadlocks as soon as one takes the two \
                 locks the other way",
                case.what
            )
        })
        .unwrap();
        unrelated.commit().await.unwrap();

        // The mutation's own record, once it is let through. A case whose
        // mutation recorded nothing would satisfy every assertion above by
        // never taking the counter at all.
        let before = current_position(&pool).await.unwrap();
        holder.rollback().await.unwrap();
        mutation.await.unwrap();
        assert!(
            current_position(&pool).await.unwrap() > before,
            "`{}` recorded no listing change, so nothing about it reaches a sidebar",
            case.what
        );
    }
}

// =========================================================================
// The cost, measured
// =========================================================================

/// Where a writer allocates its position, relative to the rest of its
/// transaction. `Last` is the invariant `record_change` documents; `Early`
/// exists only to price it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Counter {
    Absent,
    Early,
    Last,
}

/// The transaction being priced.
///
/// `Turn` is the yardstick the other shapes are compared against — a shorter
/// transaction holds the contended row for a smaller fraction of its length,
/// changing the counter's relative cost.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// Project the turn row, then reopen the Task.
    Turn,
    /// Rewrite a value the sidebar shows: `update_task`, and by the same
    /// statement the close, the reopen, the visibility change and the expiry.
    Rename,
    /// A person writes: one insert into `task_inbox`, and nothing else.
    Message,
}

impl Shape {
    fn label(self) -> &'static str {
        match self {
            Shape::Turn => "a turn ends      ",
            Shape::Rename => "a rename         ",
            Shape::Message => "a person writes  ",
        }
    }
}

/// Drive one shape from `WRITERS` connections at once. Each writer works on its
/// own Task, so the only row any two of them share is the counter.
async fn drive(pool: &Arc<PgPool>, shape: Shape, counter: Counter) -> Duration {
    let started = Instant::now();
    let mut handles = Vec::new();
    for writer in 0..WRITERS {
        let pool = Arc::clone(pool);
        handles.push(tokio::spawn(async move {
            let task_id = format!("task-{writer}");
            for step in 0..PER_WRITER {
                let mut tx = pool.begin().await.unwrap();
                if shape == Shape::Turn {
                    sqlx::query(
                        r#"INSERT INTO task_turns (task_id, turn_number, status, started_at)
                           VALUES ($1, $2, 'active', NOW())
                           ON CONFLICT (task_id, turn_number) DO UPDATE
                             SET status = 'completed', ended_at = NOW()"#,
                    )
                    .bind(&task_id)
                    .bind(step as i32)
                    .execute(&mut *tx)
                    .await
                    .unwrap();
                }
                if counter == Counter::Early {
                    record_change(&mut tx, &task_id).await.unwrap();
                }
                match shape {
                    Shape::Turn => {
                        sqlx::query(
                            "UPDATE tasks SET status = 'open', updated_at = NOW() \
                             WHERE task_id = $1 AND status <> 'closed'",
                        )
                        .bind(&task_id)
                        .execute(&mut *tx)
                        .await
                        .unwrap();
                    }
                    Shape::Rename => {
                        sqlx::query(
                            "UPDATE tasks SET name = $2, updated_at = NOW() WHERE task_id = $1",
                        )
                        .bind(&task_id)
                        .bind(format!("renamed {step}"))
                        .execute(&mut *tx)
                        .await
                        .unwrap();
                    }
                    Shape::Message => {
                        sqlx::query(
                            r#"INSERT INTO task_inbox (id, task_id, text, origin)
                               VALUES (gen_random_uuid(), $1, $2, 'system')"#,
                        )
                        .bind(&task_id)
                        .bind(format!("{task_id}-{step}"))
                        .execute(&mut *tx)
                        .await
                        .unwrap();
                    }
                }
                if counter == Counter::Last {
                    record_change(&mut tx, &task_id).await.unwrap();
                }
                tx.commit().await.unwrap();
            }
        }));
    }
    for handle in handles {
        handle.await.unwrap();
    }
    started.elapsed()
}

/// One Task per writer, so the `UPDATE tasks` above matches a row and takes the
/// lock it takes in production.
async fn seed_tasks(pool: &PgPool) {
    for writer in 0..WRITERS {
        sqlx::query(
            r#"INSERT INTO tasks (task_id, owner_user_id, created_at, last_message_at,
                                  agent_profile, repository_id, status, total_cost_usd)
               VALUES ($1, 'user-1', NOW(), NOW(), 'claude_code', 'repo-a', 'open', 0)"#,
        )
        .bind(format!("task-{writer}"))
        .execute(pool)
        .await
        .unwrap();
    }
}

/// What the counter costs under concurrent mutation, measured not assumed.
///
/// Drives many writers and compares three throughputs — no counter, counter
/// allocated last, counter allocated early — asserting only an
/// order-of-magnitude cost and that allocating last is no slower than early.
#[sqlx::test]
async fn the_counter_is_measured_under_concurrent_mutation(pool: PgPool) {
    let total = (WRITERS * PER_WRITER) as f64;
    seed_tasks(&pool).await;
    let wide = Arc::new(wide_pool(&pool, WRITERS as u32 + 4).await);

    // Establish every connection first. Without this the first arm measured
    // pays for 32 connection handshakes and reads several times slower than the
    // second — which made the counter look like a speed-up.
    drive(&wide, Shape::Turn, Counter::Absent).await;
    reset(&pool).await;

    let rate = |elapsed: Duration| total / elapsed.as_secs_f64();
    println!(
        "counter contention: {WRITERS} concurrent writers x {PER_WRITER} mutations = {total:.0}"
    );
    let mut counted = 0i64;

    for shape in [Shape::Turn, Shape::Rename, Shape::Message] {
        let without = drive(&wide, shape, Counter::Absent).await;
        reset(&pool).await;
        let last = drive(&wide, shape, Counter::Last).await;
        reset(&pool).await;
        let early = drive(&wide, shape, Counter::Early).await;
        reset(&pool).await;
        counted += 2 * (WRITERS * PER_WRITER) as i64;

        println!(
            "{}  no counter: {:.0}/s   counter last: {:.0}/s ({:.2}x)   \
             counter early: {:.0}/s ({:.2}x)",
            shape.label(),
            rate(without),
            rate(last),
            rate(without) / rate(last),
            rate(early),
            rate(without) / rate(early),
        );

        assert!(
            rate(last) > rate(without) / 10.0,
            "the counter cost more than an order of magnitude of throughput on \
             `{}` ({:.0}/s against {:.0}/s); the feed's contents need re-examining",
            shape.label().trim(),
            rate(last),
            rate(without)
        );
        // A floor, not the finding: the two orders are close enough that a
        // strict comparison would flake. What this catches is allocating early
        // becoming dramatically cheaper, which would mean the reasoning above is
        // wrong.
        assert!(
            rate(last) > rate(early) * 0.9,
            "allocating last measured materially slower than allocating early on \
             `{}` ({:.0}/s against {:.0}/s)",
            shape.label().trim(),
            rate(last),
            rate(early)
        );
    }

    // Two counted arms per shape, so the counter advanced twice per mutation in
    // each. The positions are not reset between shapes — the counter is the
    // deployment's, and nothing may ever rewind it.
    assert_eq!(current_position(&pool).await.unwrap(), counted);
}

/// Clear what one arm wrote, so the next can write it again. The counter and
/// its records are deliberately left alone.
async fn reset(pool: &PgPool) {
    sqlx::query("TRUNCATE task_turns, task_inbox CASCADE")
        .execute(pool)
        .await
        .unwrap();
}
