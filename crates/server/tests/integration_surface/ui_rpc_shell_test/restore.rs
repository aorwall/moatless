//! The database changing underneath a deployment.
//!
//! The spec's Decision 7 is two guards, and the whole of it is that neither is
//! redundant. A position higher than any this deployment has reached is
//! impossible; a database that is not the one this process started against makes
//! every position it ever handed out mean something else. Each has a case the
//! other cannot see, and the two tests below are those two cases — written so
//! that deleting either guard turns exactly one of them red.
//!
//! Both are operational disasters, so both are built rather than described. The
//! counter is really reset; the restored database is really a second database,
//! created in the cluster and migrated, with positions really reissued to
//! different changes. A test that claims to simulate a restore and does not is
//! worse than no test, because it will be cited.

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

/// A backup, restored into a new database beside the original.
///
/// `CREATE DATABASE ... TEMPLATE` is a byte-for-byte copy, and that is what
/// makes this a restore rather than a database that merely looks like one:
/// **every row the origin holds at this moment is in the copy.** Anything a
/// design were tempted to keep the environment's identity in — a UUID seeded by
/// a migration, a row written at first boot — is copied along with the rest and
/// reports the copy as the original. A restore is not distinguishable from
/// inside the data, which is why [`UiRpcEnvironmentIdentity`] is not in it.
///
/// The copy is made **in the same cluster**, so the cluster's system identifier
/// is equal on both sides too. Everything these tests catch is therefore caught
/// by the database's OID alone, which is the half of the identity easiest to
/// argue away as decoration.
///
/// Postgres refuses to copy a template anything else is connected to, so the
/// deployment's own pool is closed for the moment the copy takes and reopened
/// as [`RestoredDatabase::origin`]. That is also what a restore looks like.
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
/// Postgres rather than through [`UiRpcEnvironmentIdentity`], so a test can say which
/// half of the identity it is exercising.
async fn cluster_and_oid(pool: &PgPool) -> (String, i64) {
    sqlx::query_as(
        "SELECT (SELECT system_identifier::text FROM pg_control_system()),
                (SELECT oid::int8 FROM pg_database WHERE datname = current_database())",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The identity is a property of the database and of nothing else: it does not
/// move between reads or between connections, and a restored copy does not
/// share it.
///
/// The first half is what stops the guard firing on its own deployment — an
/// identity that varied by connection would rebuild every client every poll.
/// The second half is the half the guard is for, and it is asserted against a
/// copy that is **identical in every row** and sits in the **same cluster**, so
/// neither the data nor the system identifier is carrying it. The database's
/// OID is the only thing left, and it is enough.
///
/// # The privilege this needs, asserted rather than assumed
///
/// `the_identity_is_readable_by_a_least_privilege_application_role` below is the
/// companion this one cannot be: every `#[sqlx::test]` connects as the role that
/// created the database, which on any ordinary harness is a superuser. A
/// statement that silently required superuser would pass every test in this file
/// and fail at startup on a least-privilege deployment.
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

/// The identity statement runs as an ordinary application role.
///
/// `current_head` reads the cluster's system identifier through
/// `pg_control_system()`, and `router_from_env` returns `Err` rather than
/// falling back to a constant when that read fails — deliberately, because a
/// constant identity is an identity that never changes when the database does.
/// The consequence is that a `pg_control_system()` an ordinary role could not
/// execute would take the T3 UI RPC router down at startup on exactly the
/// deployments that are configured correctly, and no other test in this suite
/// could see it: `#[sqlx::test]` connects as the role that created the
/// database, which is a superuser on every harness this runs on.
///
/// So a role is made here that has **one** privilege in this database — `SELECT`
/// on the counter table — and the statement is run as it. Anything else the
/// statement comes to need shows up here as a `permission denied`, naming the
/// object.
///
/// The empirical answer, for the record: `pg_control_system()` is never
/// `REVOKE`d at `initdb`. Its `pg_proc.proacl` is null on PostgreSQL 13, 16 and
/// 17, which is the default — `EXECUTE` to `PUBLIC`. The functions that *are*
/// restricted (`pg_ls_logdir`, `pg_read_file`, `pg_stat_file`, `pg_ls_dir`,
/// `pg_switch_wal`) carry an explicit `{postgres=X/postgres}` and a matching
/// `REVOKE EXECUTE ... FROM public` in the image's `system_functions.sql`;
/// `pg_control_system` appears in neither file.
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
             so `router_from_env` fails to start on a least-privilege \
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

/// **Acceptance criterion 4, and the case only the position guard sees.**
///
/// The counter is reset to zero on a database that is otherwise untouched. Its
/// cluster identifier and its OID are exactly what they were — asserted below,
/// because that assertion is the point: the identity guard's inputs are
/// identical to a healthy deployment's, so it provably contributes nothing here
/// and the position guard is carrying this alone.
///
/// Verified by deleting the `after > head` comparison from `task_listing::resume`
/// and watching this fail: the client is admitted at the position it holds, the
/// replay reads an emptied table, and it is handed `[]` — told it is up to date
/// while the deployment it is watching has started counting again from one.
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

/// **Acceptance criteria 2 and 3, and the case only the identity guard sees.**
///
/// The sequence is the spec's own, built rather than described:
///
/// 1. The deployment runs, and a backup is taken at position 100.
/// 2. It runs on to position 200. A client reads the listing there and holds
///    200 — a listing that includes a conversation started after the backup.
/// 3. The database is lost and the backup is restored beside it. The copy is
///    back at 100, and it counts forward again: positions 101 through 200 are
///    **handed out a second time, to entirely different changes**.
/// 4. The client comes back holding 200.
///
/// Every number now looks right, which is the whole difficulty — the assertion
/// below proves it by asking `resume` what it would say if the two identities
/// matched, and it says replay. So without the identity guard the client is
/// told what changed *after* 200 and folds it into the listing it already had:
/// it keeps a conversation the restore has never heard of, and never hears
/// about the hundred changes that were reissued underneath it. That is
/// criterion 3's "silently skipping the changes in between", exactly.
///
/// Verified by deleting the identity comparison from `task_listing::resume` and
/// watching this fail: `values` came back as one `thread-upserted` for
/// `t-after-restore`, and `assert_decodable_snapshot_item` reported `kind` as
/// `thread-upserted` where the contract's rebuild member was required.
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

/// **Acceptance criterion 2, for the client that never reconnects.**
///
/// A subscription that was open when the database was replaced is the one path
/// no resume guard covers, because it does not resume: the socket is fine, the
/// cursor is where it was, and every poll asks for everything past a position
/// the restore has not reached yet. It is answered with silence, and then —
/// once the restore counts past that position — with changes, having skipped
/// everything in between.
///
/// Verified by deleting the identity check from `ShellFeed::poll` and watching
/// this fail on the first assertion: the poll returned `[]`, which the
/// subscription reads as "nothing changed" and sends nowhere. The sidebar is
/// then frozen for as long as the connection lasts, while the client renders
/// itself live — which is the sentence this ticket is named after.
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
/// T3 keys everything it caches for an environment on `environmentId`
/// (`client-runtime/src/platform/persistence.ts:59-63`), so this string is what
/// makes a client with a stale listing discard it without being told to. A
/// deployment that reported a configured constant here would present a replaced
/// database as the same environment, and the cached listing would survive the
/// thing it describes.
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
