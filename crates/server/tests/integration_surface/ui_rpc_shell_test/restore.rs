//! The database changing underneath a deployment.
//!
//! See the comment below for Decision 7's two guards and why the two tests
//! below build real operational disasters instead of simulating them.

use super::contract::{DESCRIPTOR_REQUIRED, assert_decodable_snapshot_item, assert_keys};
use super::feed::{delivered, one_row, other_peoples_work, reconnect_to, start_turn, watching};
use super::harness::{head, mounted_router, reads, thread_ids, viewer};
use super::seeding::{seed_live_project, seed_task};
use crate::common::body_json;
use axum::http::StatusCode;
use moatless_t3code::listing::UiRpcEnvironmentIdentity;
use moatless_t3code::listing::UiRpcResume;
use moatless_t3code::listing::resume;
use serde_json::Value;
use sqlx::AssertSqlSafe;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tower::ServiceExt;

// =========================================================================
// The database changing underneath a deployment
// =========================================================================
//
// The spec's Decision 7 is two guards, and the whole of it is that neither is
// redundant. A position higher than any this deployment has reached is
// impossible; a database that is not the one this process started against makes
// every position it ever handed out mean something else. Each has a case the
// other cannot see, and the two tests below are those two cases — written so
// that deleting either guard turns exactly one of them red.
//
// Both are operational disasters, so both are built rather than described. The
// counter is really reset; the restored database is really a second database,
// created in the cluster and migrated, with positions really reissued to
// different changes. A test that claims to simulate a restore and does not is
// worse than no test, because it will be cited.

/// A backup restored into a new database beside the original, a byte-for-byte
/// `CREATE DATABASE ... TEMPLATE` copy indistinguishable from inside the data.
///
/// Made in the same cluster, so only the OID differs; the origin pool closes
/// for the copy and reopens as [`RestoredDatabase::origin`].
pub(super) struct RestoredDatabase {
    /// The database the deployment started against, reopened after the copy.
    pub(super) origin: PgPool,
    /// The restore, which the deployment is then pointed at.
    pub(super) pool: PgPool,
    name: String,
    cluster: PgPool,
}

impl RestoredDatabase {
    pub(super) async fn discard(self) {
        let name = self.name;
        self.origin.close().await;
        self.pool.close().await;
        sqlx::query(AssertSqlSafe(format!(
            r#"DROP DATABASE IF EXISTS "{name}""#
        )))
        .execute(&self.cluster)
        .await
        .unwrap();
        self.cluster.close().await;
    }
}

pub(super) async fn back_up_and_restore(pool: PgPool) -> RestoredDatabase {
    let options = pool.connect_options().as_ref().clone();
    let current: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .unwrap();
    // Postgres truncates an identifier at 63 bytes and `#[sqlx::test]` names its
    // databases right up to that limit, so a plain suffix would truncate back to
    // the name it was derived from — and the `DROP` below would be the test
    // dropping the database it is restoring.
    let name = format!("{}_restored", &current[..current.len().min(53)]);
    assert_ne!(name, current);

    pool.close().await;
    let cluster = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone().database("postgres"))
        .await
        .unwrap();
    for statement in [
        format!(r#"DROP DATABASE IF EXISTS "{name}""#),
        format!(r#"CREATE DATABASE "{name}" TEMPLATE "{current}""#),
    ] {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&cluster)
            .await
            .unwrap();
    }

    let reopen = |options| async move {
        PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .unwrap()
    };
    RestoredDatabase {
        origin: reopen(options.clone()).await,
        pool: reopen(options.database(&name)).await,
        name,
        cluster,
    }
}

/// The cluster and the database a pool is connected to, read straight from
/// Postgres rather than through [`UiRpcEnvironmentIdentity`], so a test can say
/// which half of the identity it is exercising.
async fn cluster_and_oid(pool: &PgPool) -> (String, i64) {
    sqlx::query_as(
        "SELECT (SELECT system_identifier::text FROM pg_control_system()),
                (SELECT oid::int8 FROM pg_database WHERE datname = current_database())",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The identity is a property of the database and of nothing else: stable
/// across connections, and different for a restored copy.
///
/// The database's OID is the only signal, since a restored copy is identical in
/// every row and shares the cluster's system identifier.
#[sqlx::test]
async fn the_environment_identity_is_stable_and_names_one_database(pool: PgPool) {
    // Something in the tables, so "identical in every row" is a claim with
    // content rather than two empty schemas agreeing.
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    other_peoples_work(&pool, 20).await;

    let restored = back_up_and_restore(pool).await;
    let origin = &restored.origin;

    assert_eq!(head(origin).await.identity, head(origin).await.identity);

    // A second pool, so a second connection, on the same database.
    let second = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(origin.connect_options().as_ref().clone())
        .await
        .unwrap();
    assert_eq!(
        head(origin).await.identity,
        head(&second).await.identity,
        "the identity moved between two connections to the same database"
    );
    second.close().await;

    // The copy really is a copy: same counter, same records, same rows.
    assert_eq!(
        head(&restored.pool).await.position,
        head(origin).await.position
    );
    assert_eq!(
        thread_ids(&restored.pool, &viewer("user-1")).await,
        thread_ids(origin, &viewer("user-1")).await
    );

    let (origin_cluster, origin_oid) = cluster_and_oid(origin).await;
    let (restored_cluster, restored_oid) = cluster_and_oid(&restored.pool).await;
    assert_eq!(
        origin_cluster, restored_cluster,
        "the premise failed: the restore is meant to be in the same cluster"
    );
    assert_ne!(origin_oid, restored_oid);

    // Both halves, pinned against the values Postgres reports for them.
    //
    // The OID half is what the restore below exercises. The cluster half
    // cannot be — a second cluster is not something `#[sqlx::test]` can
    // produce — and it is the half most easily mistaken for decoration, so it
    // is pinned structurally instead. Dropping it would leave the identity
    // resting on a per-cluster counter that starts at 16,384 in every cluster
    // alive, and a restore into a freshly-`initdb`'d server would present as
    // the database it replaced.
    assert_eq!(
        head(origin).await.identity,
        UiRpcEnvironmentIdentity::new(format!("{origin_cluster}:{origin_oid}")),
        "the identity is no longer the cluster's system identifier and the \
         database's OID"
    );

    assert_ne!(
        head(origin).await.identity,
        head(&restored.pool).await.identity,
        "a restored copy reported the same identity as its original; every \
         position the original ever issued now looks valid in the copy"
    );

    restored.discard().await;
}

/// The identity statement must run as an ordinary application role, not just
/// the superuser every `#[sqlx::test]` connects as.
///
/// A role with only `SELECT` on the counter table runs it here; anything else
/// the statement needs shows up as `permission denied`, naming the object.
#[sqlx::test]
async fn the_identity_is_readable_by_a_least_privilege_application_role(pool: PgPool) {
    // Something for the counter to have counted, so the position read is not
    // trivially satisfied by an empty table.
    seed_live_project(&pool).await;

    let options = pool.connect_options().as_ref().clone();
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .unwrap();
    // Cluster-wide and therefore shared with every other test running against
    // this server, so the name is derived from this test's own database.
    let role = format!("{}_least", &database[..database.len().min(50)]);

    for statement in [
        format!(r#"DROP ROLE IF EXISTS "{role}""#),
        format!(
            r#"CREATE ROLE "{role}" LOGIN PASSWORD 'least' NOSUPERUSER NOCREATEDB NOCREATEROLE"#
        ),
        // The whole grant. Not `pg_read_all_stats`, not `pg_monitor`, not the
        // database owner: if the identity read needed any of those, this is
        // where it would say so.
        format!(r#"GRANT SELECT ON task_listing_position TO "{role}""#),
    ] {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&pool)
            .await
            .unwrap();
    }

    let least = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.username(&role).password("least"))
        .await
        .unwrap();
    let ordinary = reads(&least).listing_head().await;
    least.close().await;

    // Cleaned up before the assertion, so a failure does not leave a role
    // behind for the next run to trip over.
    for statement in [
        format!(r#"DROP OWNED BY "{role}""#),
        format!(r#"DROP ROLE IF EXISTS "{role}""#),
    ] {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&pool)
            .await
            .unwrap();
    }

    let ordinary = ordinary.unwrap_or_else(|error| {
        panic!(
            "an ordinary application role cannot read the environment identity, \
             so `router_if_enabled` fails to start on a least-privilege \
             deployment: {error}"
        )
    });
    assert_eq!(
        ordinary,
        head(&pool).await,
        "the identity a least-privilege role reads is not the one the \
         deployment believes it is serving"
    );
}

/// Acceptance criterion 4, the case only the position guard sees: the counter
/// resets to zero on an otherwise untouched database.
///
/// The identity guard's inputs are unchanged, so it provably contributes
/// nothing here; deleting `after > head` from `task_listing::resume` makes this
/// fail.
#[sqlx::test]
async fn a_counter_reset_on_the_same_database_is_caught_by_the_position_guard(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");
    other_peoples_work(&pool, 200).await;

    let before = head(&pool).await;
    let held = before.position;
    assert!(held > 0, "the premise failed: nothing produced a position");

    // The counter goes back to zero, and the records it issued go with it —
    // which is what a reset has to look like, since `task_listing_changes`
    // keys on the position and a counter starting again would collide.
    sqlx::query("DELETE FROM task_listing_position")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("TRUNCATE task_listing_changes")
        .execute(&pool)
        .await
        .unwrap();

    let after = head(&pool).await;
    assert_eq!(
        after.position, 0,
        "the premise failed: the counter did not reset"
    );
    assert_eq!(
        after.identity, before.identity,
        "the premise failed: resetting the counter changed the database's \
         identity, so this is not a case only the position guard can see"
    );

    let (values, mut feed) = reconnect_to(&pool, &before.identity, &viewer, Some(held)).await;
    assert_eq!(
        values.len(),
        1,
        "a client holding {held} on a deployment that has reset to zero was not \
         given a listing: {values:#?}"
    );
    assert_decodable_snapshot_item(&values[0]);
    assert_eq!(values[0]["snapshot"]["snapshotSequence"], 0);

    // And it is watching again rather than parked above a counter that will
    // take 200 changes to reach it: the next thing that happens arrives.
    start_turn(&pool, "t-live", "run-1").await;
    let event = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(event["thread"]["id"], "t-live");
    assert_eq!(event["sequence"], 1);
}

/// Acceptance criteria 2 and 3, the case only the identity guard sees: a
/// restored backup reissues positions to entirely different changes than a
/// stale cursor remembers.
///
/// Without the identity guard, `resume` treats the matching positions as valid
/// and replays instead of rebuilding, silently skipping every reissued change —
/// criterion 3's failure exactly.
#[sqlx::test]
async fn a_restored_backup_reissuing_positions_is_caught_by_the_identity_guard(pool: PgPool) {
    let viewer = viewer("user-1");

    // 1. The deployment, and the backup.
    seed_live_project(&pool).await;
    seed_task(&pool, "t-before-backup", "user-1", Some("ws-live")).await;
    other_peoples_work(&pool, 100).await;
    let restored = back_up_and_restore(pool).await;
    let backed_up_at = head(&restored.pool).await.position;

    // 2. The original runs on, and the client reads the listing there.
    seed_task(
        &restored.origin,
        "t-after-backup",
        "user-1",
        Some("ws-live"),
    )
    .await;
    other_peoples_work(&restored.origin, 100).await;
    let original = head(&restored.origin).await;
    let held = original.position;
    assert!(
        held > backed_up_at,
        "the premise failed: the backup is not behind"
    );
    assert!(
        thread_ids(&restored.origin, &viewer)
            .await
            .contains(&"t-after-backup".to_string()),
        "the premise failed: the client's listing does not hold anything the \
         backup is missing"
    );

    // 3. The restore counts forward again, reissuing 101..=200 to other work.
    seed_task(&restored.pool, "t-after-restore", "user-1", Some("ws-live")).await;
    let reissued = held - head(&restored.pool).await.position;
    other_peoples_work(&restored.pool, reissued).await;
    start_turn(&restored.pool, "t-after-restore", "run-1").await;
    let now = head(&restored.pool).await;
    assert!(
        reissued > 0 && now.position > held,
        "the premise failed: the restore did not reissue the client's positions"
    );

    // 4. Every other guard admits this cursor, so anything that refuses it is
    //    the identity.
    assert_eq!(
        resume(&now.identity, &now.identity, now.position, held),
        UiRpcResume::From(held),
        "the premise failed: a guard other than the identity already refuses \
         this cursor, so this test is not about the identity guard"
    );

    let (values, _feed) =
        reconnect_to(&restored.pool, &original.identity, &viewer, Some(held)).await;
    assert_eq!(
        values.len(),
        1,
        "a client resuming into a restored database was replayed from a position \
         that now belongs to different changes: {values:#?}"
    );
    assert_eq!(
        values[0]["kind"],
        "snapshot",
        "the client was replayed a difference against a listing it built in a \
         database that no longer exists; positions {} to {held} were handed out \
         a second time and it will never hear about them: {values:#?}",
        backed_up_at + 1
    );
    assert_decodable_snapshot_item(&values[0]);

    // The listing it is given is the restore's, whole. The client replaces what
    // it holds with this, so the conversation that only ever existed after the
    // backup goes rather than lingering forever in a sidebar nothing updates.
    let mut threads: Vec<&str> = values[0]["snapshot"]["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|thread| thread["id"].as_str().unwrap())
        .collect();
    threads.sort_unstable();
    assert_eq!(threads, vec!["t-after-restore", "t-before-backup"]);
    assert_eq!(values[0]["snapshot"]["snapshotSequence"], now.position);

    restored.discard().await;
}

/// Acceptance criterion 2, for the client that never reconnects: a live
/// subscription open when the database was replaced resumes nothing, so no
/// resume guard covers it.
///
/// Every poll asks past a position the restore hasn't reached, so it is
/// answered with silence and then, once the restore catches up, with changes —
/// having silently skipped everything in between.
#[sqlx::test]
async fn a_connected_client_whose_database_is_replaced_rebuilds_rather_than_stalling(pool: PgPool) {
    let viewer = viewer("user-1");
    seed_live_project(&pool).await;
    seed_task(&pool, "t-before-backup", "user-1", Some("ws-live")).await;
    other_peoples_work(&pool, 100).await;

    let restored = back_up_and_restore(pool).await;
    other_peoples_work(&restored.origin, 100).await;

    // Watching, and up to date, on the database as it was.
    let mut feed = watching(&restored.origin, &viewer).await;
    let held = head(&restored.origin).await.position;
    assert!(
        delivered(&restored.origin, &mut feed, &viewer)
            .await
            .is_empty()
    );

    // The deployment is repointed at the restore, which is behind the cursor.
    seed_task(&restored.pool, "t-after-restore", "user-1", Some("ws-live")).await;
    assert!(
        head(&restored.pool).await.position < held,
        "the premise failed: the restore has already passed the cursor, so a \
         stall would not be observable"
    );

    let items = delivered(&restored.pool, &mut feed, &viewer).await;
    assert_eq!(
        items.len(),
        1,
        "a subscription whose database was replaced was told nothing: {items:#?}"
    );
    assert_eq!(
        items[0]["kind"], "snapshot",
        "a subscription whose database was replaced was sent a difference \
         against the listing it holds: {items:#?}"
    );
    assert_decodable_snapshot_item(&items[0]);
    let mut threads: Vec<&str> = items[0]["snapshot"]["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|thread| thread["id"].as_str().unwrap())
        .collect();
    threads.sort_unstable();
    assert_eq!(threads, vec!["t-after-restore", "t-before-backup"]);

    // The same feed then carries on in the restore rather than rebuilding every
    // poll: it took the new identity and the new position with it.
    assert!(
        delivered(&restored.pool, &mut feed, &viewer)
            .await
            .is_empty()
    );
    start_turn(&restored.pool, "t-after-restore", "run-1").await;
    let event = one_row(&restored.pool, &mut feed, &viewer).await;
    assert_eq!(event["thread"]["id"], "t-after-restore");

    restored.discard().await;
}

/// The public descriptor, as a client reads it before any login.
async fn descriptor(pool: &PgPool) -> Value {
    let router = mounted_router(pool).await;
    let response = router
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/.well-known/t3/environment")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    body_json(response).await
}

/// The identity half of Decision 7, on the wire.
///
/// T3 keys its cache on `environmentId` (`persistence.ts:59-63`), so a
/// configured constant here would present a replaced database as the same
/// environment and let the stale cached listing survive it.
#[sqlx::test]
async fn the_environment_descriptor_reports_the_database_it_is_serving(pool: PgPool) {
    let published = descriptor(&pool).await;
    assert_keys(
        &published,
        "ExecutionEnvironmentDescriptor",
        DESCRIPTOR_REQUIRED,
        &[],
    );
    let reported = published["environmentId"].as_str().unwrap().to_string();
    assert_ne!(
        reported, "moatless",
        "the environment id is still the configured constant, so no database \
         change can move it"
    );
    assert!(
        reported.starts_with("moatless-"),
        "the configured name is no longer the readable half of {reported}"
    );
    assert_eq!(
        descriptor(&pool).await["environmentId"],
        reported,
        "the environment id moved without the database moving; every client \
         would discard its cached listing on every restart"
    );

    let restored = back_up_and_restore(pool).await;
    assert_eq!(
        descriptor(&restored.origin).await["environmentId"],
        Value::String(reported.clone()),
        "the environment id moved when nothing about the database did"
    );
    assert_ne!(
        descriptor(&restored.pool).await["environmentId"],
        Value::String(reported),
        "a restored copy of this database presents as the same environment, so \
         a client keeps the listing it built against the original"
    );
    restored.discard().await;
}
