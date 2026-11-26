//! A migration that needs a table some long read still holds: the other reads
//! of that table keep answering while it waits, and it lands once the read ends.

use std::time::{Duration, Instant};

use sqlx::migrate::{Migrate, MigrateError, Migration, MigrationType, Migrator};
use sqlx::{PgPool, SqlSafeStr};

use super::{bounded_lock_waits, is_lock_timeout, run_retrying_locks};

const LOCK_TIMEOUT: Duration = Duration::from_millis(200);
const PAUSE: Duration = Duration::from_millis(200);

/// Longer than any wait a bounded migration can impose, so a read that waits
/// this long proves it queued for the whole hold.
const HOLD: Duration = Duration::from_secs(3);

fn adding_a_column() -> Migrator {
    let migrator = Migrator::with_migrations(vec![Migration::new(
        1,
        "add a column".into(),
        MigrationType::Simple,
        sqlx::AssertSqlSafe("ALTER TABLE notes ADD COLUMN body text").into_sql_str(),
        false,
    )]);
    let mut bounded = bounded_lock_waits(&migrator, LOCK_TIMEOUT);
    // CI's test databases copy a fully migrated `template1`, whose
    // `_sqlx_migrations` rows this one-migration source would report missing.
    bounded.dangerous_set_table_name("_lock_test_migrations");
    bounded
}

async fn seed(pool: &PgPool) {
    sqlx::raw_sql("CREATE TABLE notes (id int); INSERT INTO notes SELECT generate_series(1, 100)")
        .execute(pool)
        .await
        .expect("seed notes");
}

/// Open a transaction that has read `notes` and so holds its `ACCESS SHARE`
/// lock until the returned task commits after `hold`.
async fn hold_a_read(pool: &PgPool, hold: Duration) -> tokio::task::JoinHandle<()> {
    let mut tx = pool.begin().await.expect("begin");
    sqlx::query("SELECT count(*) FROM notes")
        .execute(&mut *tx)
        .await
        .expect("read notes");
    tokio::spawn(async move {
        tokio::time::sleep(hold).await;
        tx.commit().await.expect("commit");
    })
}

#[sqlx::test(migrations = false)]
async fn reads_keep_answering_while_a_migration_waits_for_a_held_table(pool: PgPool) {
    seed(&pool).await;
    let held = hold_a_read(&pool, HOLD).await;
    let mut conn = pool.acquire().await.expect("acquire");
    let migrator = adding_a_column();
    let migrating = run_retrying_locks(&migrator, &mut conn, 100, PAUSE);
    let reading = async {
        tokio::time::sleep(LOCK_TIMEOUT / 2).await;
        let mut slowest = Duration::ZERO;
        let reading = Instant::now();
        while reading.elapsed() < HOLD / 2 {
            let started = Instant::now();
            sqlx::query("SELECT count(*) FROM notes")
                .execute(&pool)
                .await
                .expect("read notes");
            slowest = slowest.max(started.elapsed());
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        slowest
    };

    let (migrated, slowest) = tokio::join!(migrating, reading);

    assert!(
        slowest < LOCK_TIMEOUT * 3,
        "a read waited {slowest:?} behind the migration"
    );
    held.await.expect("held read");
    migrated.expect("the migration lands once the read ends");
    let column: Option<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'notes' AND column_name = 'body'",
    )
    .fetch_optional(&pool)
    .await
    .expect("read columns");
    assert_eq!(column.as_deref(), Some("body"));
}

#[sqlx::test(migrations = false)]
async fn a_table_held_past_every_attempt_fails_with_the_lock_error(pool: PgPool) {
    seed(&pool).await;
    let held = hold_a_read(&pool, HOLD).await;

    let mut conn = pool.acquire().await.expect("acquire");
    let result = run_retrying_locks(&adding_a_column(), &mut conn, 2, PAUSE).await;

    match result {
        Err(MigrateError::ExecuteMigration(err, 1)) => assert!(is_lock_timeout(&err), "{err}"),
        other => panic!("expected the lock timeout, got {other:?}"),
    }
    held.await.expect("held read");
}

/// A retried attempt must leave sqlx's migration lock as it found it, or a
/// pooled connection keeps holding it and blocks every later migrator.
#[sqlx::test(migrations = false)]
async fn a_retried_migration_releases_the_migration_lock(pool: PgPool) {
    seed(&pool).await;
    let held = hold_a_read(&pool, LOCK_TIMEOUT * 4).await;
    let mut conn = pool.acquire().await.expect("acquire");

    run_retrying_locks(&adding_a_column(), &mut conn, 100, PAUSE)
        .await
        .expect("the migration lands once the read ends");
    held.await.expect("held read");

    let mut other = pool.acquire().await.expect("acquire another");
    tokio::time::timeout(Duration::from_secs(2), other.lock())
        .await
        .expect("another connection takes the migration lock")
        .expect("lock");
    other.unlock().await.expect("unlock");
}
