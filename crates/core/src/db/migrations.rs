//! Workspace database migrations. No other crate declares a migration
//! directory; this is the only one `run_migrations` runs.
//!
//! `migrations/` starts at [`BASELINE`], the squash of the chain
//! `squashed_migrations.tsv` lists. [`adopt_baseline`] moves a database that
//! applied that chain onto it.

use std::time::Duration;

use sqlx::AssertSqlSafe;
use sqlx::Connection;
use sqlx::PgConnection;
use sqlx::SqlSafeStr;
use sqlx::migrate::{Migrate, MigrateError, Migrator};

use super::DbPool;

/// The version of `0254_baseline.sql`, which replaces every squashed one.
const BASELINE: i64 = 254;

/// `version`, `description` and hex `checksum` of each squashed migration, as
/// `_sqlx_migrations` recorded them. Never edit it: adoption compares a
/// database's rows against it byte for byte.
const SQUASHED: &str = include_str!("squashed_migrations.tsv");

/// How long one transactional migration waits for a lock before it rolls back
/// and is retried.
///
/// Every query on the table queues behind the waiting `ALTER TABLE`, so this
/// stays below the pool's 5 s `acquire_timeout` rather than letting the
/// serving replica's requests time out, as 0224's 12 s wait did.
const LOCK_TIMEOUT: Duration = Duration::from_secs(2);

/// Attempts at the pending migrations before startup fails on the lock.
const LOCK_ATTEMPTS: u32 = 15;

/// Pause between attempts, so the queued queries drain before the next wait.
const LOCK_RETRY_PAUSE: Duration = Duration::from_secs(2);

/// Run pending SQL migrations from `./migrations/` after [`adopt_baseline`],
/// under sqlx's advisory migration lock, so replicas run one at a time.
///
/// A schema other than `public` (an isolated E2E run, chosen by `search_path`
/// in `DATABASE_URL`) gets its SQL through [`scope_to_current_schema`].
/// Returns the lock error once [`LOCK_ATTEMPTS`] have each waited
/// [`LOCK_TIMEOUT`].
pub async fn run_migrations(pool: &DbPool) -> Result<(), MigrateError> {
    let mut migrator = sqlx::migrate!("./migrations");
    let mut conn = pool.acquire().await?;
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(&mut *conn)
        .await?;
    if schema != "public" {
        tracing::info!(schema, "Migrating an isolated schema");
        migrator = isolated(&migrator);
    }
    let baseline = baseline_checksum(&migrator)?;
    let migrator = bounded_lock_waits(&migrator, LOCK_TIMEOUT);
    locked(&mut conn, async |conn| {
        adopt_baseline(conn, &baseline).await?;
        run_retrying_locks(&migrator, conn, LOCK_ATTEMPTS, LOCK_RETRY_PAUSE).await
    })
    .await
}

/// Run `migrator` on `conn`, which holds the migration lock, starting over
/// after a migration that gave up waiting for a table lock, at most
/// `attempts` times.
///
/// Safe to repeat only for a transactional migration: its rollback leaves no
/// `_sqlx_migrations` row, so the next attempt applies it whole.
async fn run_retrying_locks(
    migrator: &Migrator,
    conn: &mut PgConnection,
    attempts: u32,
    pause: Duration,
) -> Result<(), MigrateError> {
    let mut attempt = 1;
    loop {
        // The session lock nests, so `run` taking it again does not wait.
        match migrator.run(&mut *conn).await {
            Err(MigrateError::ExecuteMigration(err, version))
                if attempt < attempts && is_lock_timeout(&err) =>
            {
                tracing::warn!(
                    version,
                    attempt,
                    "A migration gave up waiting for a table lock; retrying"
                );
                // Drop the nesting the failed `run` left, or the caller's one
                // `unlock` keeps the lock held on a pooled connection.
                conn.unlock().await?;
                attempt += 1;
                tokio::time::sleep(pause).await;
            }
            result => return result,
        }
    }
}

/// `lock_not_available`, which `lock_timeout` raises.
fn is_lock_timeout(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "55P03")
}

/// Put a database adopted by [`run_migrations`] back on the squashed chain,
/// so an image built before the baseline boots against it. A no-op on a
/// database already on that chain.
///
/// Refuses when any migration after [`BASELINE`] is applied: the old image
/// would not know that schema.
pub async fn restore_squashed_history(pool: &DbPool) -> Result<(), MigrateError> {
    let migrator = sqlx::migrate!("./migrations");
    let baseline = baseline_checksum(&migrator)?;
    let mut conn = pool.acquire().await?;
    locked(&mut conn, async |conn| {
        let applied = applied(conn).await?;
        if applied.is_empty() || squashed_chain(&applied)? {
            return Ok(());
        }
        if applied != [(BASELINE, true, baseline)] {
            return Err(source(format!(
                "cannot restore the squashed history: `_sqlx_migrations` records {:?}, \
                 not only the baseline {BASELINE}",
                applied.iter().map(|(v, ..)| *v).collect::<Vec<_>>()
            )));
        }
        let mut tx = conn.begin().await?;
        sqlx::query("DELETE FROM _sqlx_migrations WHERE version = $1")
            .bind(BASELINE)
            .execute(&mut *tx)
            .await?;
        for (version, description, checksum) in squashed() {
            sqlx::query(
                "INSERT INTO _sqlx_migrations \
                 (version, description, success, checksum, execution_time) \
                 VALUES ($1, $2, true, $3, 0)",
            )
            .bind(version)
            .bind(description)
            .bind(checksum)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        tracing::warn!("Restored the squashed migration history in place of the baseline");
        Ok(())
    })
    .await
}

/// Run `work` holding sqlx's migration lock on `conn`.
///
/// On failure the connection is closed rather than unlocked: `Migrator::run`
/// returns early without releasing its own nested lock, and a pooled
/// connection still holding it would block every later migrator.
async fn locked(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
    work: impl AsyncFnOnce(&mut PgConnection) -> Result<(), MigrateError>,
) -> Result<(), MigrateError> {
    conn.lock().await?;
    let result = work(&mut *conn).await;
    match result {
        Ok(()) => conn.unlock().await,
        Err(e) => {
            conn.close_on_drop();
            Err(e)
        }
    }
}

/// Replace a complete squashed chain in `_sqlx_migrations` with the
/// [`BASELINE`] row, in one transaction, before sqlx validates the rows.
///
/// A fresh or already adopted database is left alone. One that applied only
/// part of the chain fails: its schema is neither the old head nor the
/// baseline, and guessing would let sqlx skip the rest.
async fn adopt_baseline(conn: &mut PgConnection, checksum: &[u8]) -> Result<(), MigrateError> {
    let applied = applied(conn).await?;
    if !squashed_chain(&applied)? {
        return Ok(());
    }
    let mut tx = conn.begin().await?;
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version < $1")
        .bind(BASELINE)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES ($1, 'baseline', true, $2, 0)",
    )
    .bind(BASELINE)
    .bind(checksum)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    tracing::info!(
        replaced = applied.len(),
        "Adopted the migration baseline in place of the squashed chain"
    );
    Ok(())
}

/// `(version, success, checksum)` of every `_sqlx_migrations` row, by
/// version; empty when the table does not exist yet.
async fn applied(conn: &mut PgConnection) -> Result<Vec<(i64, bool, Vec<u8>)>, MigrateError> {
    let exists: bool = sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations') IS NOT NULL")
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Ok(Vec::new());
    }
    Ok(
        sqlx::query_as("SELECT version, success, checksum FROM _sqlx_migrations ORDER BY version")
            .fetch_all(&mut *conn)
            .await?,
    )
}

/// Whether `applied` is exactly the squashed chain, every row succeeded with
/// its recorded checksum. `false` when it holds none of it.
///
/// Fails on any other overlap with the chain: some of it missing, a
/// checksum that differs, or the chain beside the baseline or a later row.
fn squashed_chain(applied: &[(i64, bool, Vec<u8>)]) -> Result<bool, MigrateError> {
    let expected: Vec<(i64, bool, Vec<u8>)> = squashed()
        .map(|(version, _, checksum)| (version, true, checksum))
        .collect();
    if !applied.iter().any(|(version, ..)| *version < BASELINE) {
        return Ok(false);
    }
    if applied == expected.as_slice() {
        return Ok(true);
    }
    let missing: Vec<i64> = expected
        .iter()
        .filter(|row| !applied.contains(row))
        .map(|(version, ..)| *version)
        .collect();
    let extra: Vec<i64> = applied
        .iter()
        .filter(|row| !expected.contains(row))
        .map(|(version, ..)| *version)
        .collect();
    Err(source(format!(
        "this database applied only part of the migration chain squashed into baseline \
         {BASELINE}: missing or modified {missing:?}, unexpected {extra:?}. Start an image \
         built before the squash once so it finishes that chain, then this one"
    )))
}

/// The squashed chain's rows, parsed from [`SQUASHED`].
fn squashed() -> impl Iterator<Item = (i64, &'static str, Vec<u8>)> {
    SQUASHED.lines().map(|line| {
        let mut fields = line.split('\t');
        let mut next = || fields.next().unwrap_or_default();
        let (version, description, checksum) = (next(), next(), next());
        (
            version.parse().unwrap_or_default(),
            description,
            (0..checksum.len())
                .step_by(2)
                .filter_map(|at| u8::from_str_radix(checksum.get(at..at + 2)?, 16).ok())
                .collect(),
        )
    })
}

fn baseline_checksum(migrator: &Migrator) -> Result<Vec<u8>, MigrateError> {
    migrator
        .iter()
        .find(|migration| migration.version == BASELINE)
        .map(|migration| migration.checksum.to_vec())
        .ok_or(MigrateError::VersionNotPresent(BASELINE))
}

fn source(message: String) -> MigrateError {
    MigrateError::Source(message.into())
}

/// `migrator` with every transactional migration's lock waits capped at
/// `timeout`.
///
/// A `no_tx` migration is left alone: a `CREATE INDEX CONCURRENTLY` that times
/// out leaves an invalid index its `IF NOT EXISTS` retry would then skip.
fn bounded_lock_waits(migrator: &Migrator, timeout: Duration) -> Migrator {
    let set = format!("SET LOCAL lock_timeout = '{}ms';\n", timeout.as_millis());
    map_sql(migrator, |migration| {
        if migration.no_tx {
            migration.sql.as_str().to_owned()
        } else {
            format!("{set}{}", migration.sql.as_str())
        }
    })
}

/// `migrator` with each migration's SQL rewritten by `rewrite`.
///
/// Each migration keeps its original checksum, so a database migrated this
/// way records the same `_sqlx_migrations` rows as the files do.
fn map_sql(migrator: &Migrator, rewrite: impl Fn(&sqlx::migrate::Migration) -> String) -> Migrator {
    Migrator::with_migrations(
        migrator
            .iter()
            .map(|migration| {
                let mut rewritten = migration.clone();
                rewritten.sql = AssertSqlSafe(rewrite(migration)).into_sql_str();
                rewritten
            })
            .collect(),
    )
}

/// `migrator` with every migration's SQL scoped to the current schema.
fn isolated(migrator: &Migrator) -> Migrator {
    map_sql(migrator, |migration| {
        scope_to_current_schema(migration.sql.as_str())
    })
}

/// Restrict the catalog probes the migrations make to `current_schema()`.
///
/// Unscoped, a probe from a fresh schema finds `public`'s tables and skips the
/// step that would create its own. Fix a probe here, never in an applied
/// migration file: production records its checksum.
fn scope_to_current_schema(sql: &str) -> String {
    let sql = insert_before_column(sql, "table_name", "table_schema = current_schema() AND ");
    insert_before_column(
        &sql,
        "conname",
        "connamespace = current_schema()::regnamespace AND ",
    )
}

/// Insert `filter` before every `WHERE <column> =`, keeping the whitespace
/// between `WHERE` and `column`.
fn insert_before_column(sql: &str, column: &str, filter: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut rest = sql;
    while let Some(at) = rest.find("WHERE") {
        let (head, tail) = rest.split_at(at + "WHERE".len());
        out.push_str(head);
        let gap = tail.len() - tail.trim_start().len();
        let after = &tail[gap..];
        let is_match = gap > 0
            && after
                .strip_prefix(column)
                .is_some_and(|next| next.trim_start().starts_with('='));
        out.push_str(&tail[..gap]);
        if is_match {
            out.push_str(filter);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two files sharing a version is not a conflict git can see: both merge,
    /// and the loser is silently skipped against a database that recorded the
    /// winner, leaving its schema change unapplied and startup refusing to
    /// boot.
    #[test]
    fn every_migration_has_its_own_version() {
        let migrations = sqlx::migrate!();
        let mut seen = std::collections::HashMap::new();

        for migration in migrations.iter() {
            if let Some(other) = seen.insert(migration.version, &migration.description) {
                panic!(
                    "migration {} is claimed by both {:?} and {:?}",
                    migration.version, other, migration.description
                );
            }
        }
    }

    /// Whether one statement, lowercased with comments stripped, removes or
    /// retypes something the previous release reads, or adds a `NOT NULL`
    /// column its inserts cannot fill.
    fn breaks_previous_release(statement: &str) -> bool {
        let words = statement.split_whitespace().collect::<Vec<_>>();
        if words
            .windows(2)
            .any(|w| matches!(w, ["drop", "table" | "view" | "materialized" | "type"]))
        {
            return true;
        }
        let Some(start) = words.windows(2).position(|w| w == ["alter", "table"]) else {
            return false;
        };
        let mut rest = &words[start + 2..];
        while let ["if" | "exists" | "only", tail @ ..] = rest {
            rest = tail;
        }
        // Split the actions after the table name on top-level commas, so a
        // `numeric(10, 2)` stays inside its action.
        let mut actions = vec![Vec::new()];
        let mut depth = 0isize;
        for word in rest.iter().skip(1) {
            depth +=
                word.matches('(').count().cast_signed() - word.matches(')').count().cast_signed();
            let action = actions.last_mut().expect("starts with one action");
            match word.strip_suffix(',') {
                Some(head) if depth == 0 => {
                    action.push(head);
                    actions.push(Vec::new());
                }
                _ => action.push(word),
            }
        }
        actions.iter().any(|action| match action.as_slice() {
            ["rename" | "drop", "constraint", ..] => false,
            ["rename" | "drop", ..] => true,
            ["alter", "column", _, next @ ..] | ["alter", _, next @ ..] => {
                matches!(next, ["type", ..] | ["set", "data", "type", ..])
            }
            [
                "add",
                "constraint" | "check" | "primary" | "unique" | "foreign" | "exclude",
                ..,
            ] => false,
            ["add", ..] => {
                action.windows(2).any(|w| w == ["not", "null"]) && !action.contains(&"default")
            }
            _ => false,
        })
    }

    /// The statements in `sql` that [`breaks_previous_release`] flags, each
    /// with its whitespace collapsed.
    fn breaking_statements(sql: &str) -> Vec<String> {
        sql.lines()
            .map(|line| line.split("--").next().unwrap_or_default())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
            .split(';')
            .filter(|statement| breaks_previous_release(statement))
            .map(|statement| statement.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect()
    }

    /// Pods of the previous release keep serving after a migration, so it must
    /// keep every table and column they read or write: 0202's rename failed
    /// each old task read. Add the new name; drop the old one in a later
    /// migration marked `-- contract:` naming the release that stopped using it.
    #[test]
    fn no_migration_breaks_the_previous_release() {
        let migrations = sqlx::migrate!();

        for migration in migrations.iter() {
            let sql = migration.sql.as_str();
            if migration.version <= BASELINE || sql.contains("-- contract:") {
                continue;
            }
            let breaking = breaking_statements(sql);
            assert!(
                breaking.is_empty(),
                "migration {} breaks the previous release: {breaking:?}",
                migration.version
            );
        }
    }

    /// 0202 and 0212 each failed the previous release's pods once applied.
    #[test]
    fn the_migrations_that_broke_the_previous_release_are_flagged() {
        let sql = [
            "ALTER TABLE tasks RENAME COLUMN inactive_ttl_days TO close_after_settled_days;\n\
             ALTER TABLE tasks ADD COLUMN settle_after_hours INTEGER;",
            "ALTER TABLE artifact_records RENAME TO collection_records;\n\
             ALTER INDEX IF EXISTS artifact_records_pkey RENAME TO collection_records_pkey;",
        ];

        for migration in sql {
            assert_eq!(breaking_statements(migration).len(), 1, "{migration}");
        }
    }

    #[test]
    fn breaking_statements_are_told_from_compatible_ones() {
        let cases = [
            ("alter table tasks rename column a to b", true),
            ("alter table tasks rename a to b", true),
            ("alter table if exists tasks rename to jobs", true),
            ("alter table tasks drop column a", true),
            ("alter table only tasks drop a", true),
            ("alter table tasks add column b int, drop column a", true),
            ("alter table tasks alter column a type bigint", true),
            ("alter table tasks alter a type bigint", true),
            (
                "alter table tasks alter column a set data type bigint",
                true,
            ),
            ("alter table tasks add column a int not null", true),
            ("drop table tasks", true),
            ("drop view task_view", true),
            ("do $$ begin alter table tasks drop column a", true),
            ("do $$ begin drop table tasks", true),
            ("alter table tasks add column a int", false),
            (
                "alter table tasks add constraint c check (a is not null)",
                false,
            ),
            ("alter table tasks add check (a is not null)", false),
            ("drop materialized view task_view", true),
            (
                "alter table tasks add column a int not null default 0",
                false,
            ),
            ("alter table tasks add column a numeric(10, 2)", false),
            (
                "alter table task_events alter column type set not null",
                false,
            ),
            ("alter table task_events alter type set default 'x'", false),
            ("alter table tasks alter column a drop default", false),
            ("alter table tasks drop constraint tasks_a_check", false),
            ("alter table tasks rename constraint a to b", false),
            ("alter index tasks_a_idx rename to tasks_b_idx", false),
            ("drop index tasks_a_idx", false),
            ("update tasks set a = 1", false),
        ];

        for (statement, breaking) in cases {
            assert_eq!(breaks_previous_release(statement), breaking, "{statement}");
        }
    }

    fn chain() -> Vec<(i64, bool, Vec<u8>)> {
        squashed()
            .map(|(version, _, checksum)| (version, true, checksum))
            .collect()
    }

    #[test]
    fn the_squashed_chain_parses_and_ends_before_the_baseline() {
        let chain = chain();
        assert_eq!(chain.len(), 251);
        assert_eq!(chain.first().map(|row| row.0), Some(1));
        assert_eq!(chain.last().map(|row| row.0), Some(BASELINE - 1));
        assert!(chain.iter().all(|(_, _, checksum)| checksum.len() == 48));
        assert!(chain.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn only_the_whole_chain_is_adopted() {
        assert!(!squashed_chain(&[]).unwrap());
        assert!(!squashed_chain(&[(BASELINE, true, vec![1])]).unwrap());
        assert!(squashed_chain(&chain()).unwrap());
    }

    #[test]
    fn a_partial_or_modified_chain_is_refused() {
        let mut partial = chain();
        partial.pop();
        let mut modified = chain();
        modified[3].2[0] ^= 1;
        let mut failed = chain();
        failed[7].1 = false;
        let mut beside_baseline = chain();
        beside_baseline.push((BASELINE, true, vec![1]));

        for applied in [partial, modified, failed, beside_baseline] {
            let error = squashed_chain(&applied).unwrap_err().to_string();
            assert!(
                error.contains("only part of the migration chain"),
                "{error}"
            );
        }
    }

    #[test]
    fn the_baseline_is_the_only_migration_and_runs_in_a_transaction() {
        let migrations = sqlx::migrate!();
        let versions: Vec<(i64, bool)> = migrations.iter().map(|m| (m.version, m.no_tx)).collect();
        assert_eq!(versions.first(), Some(&(BASELINE, false)));
    }

    #[test]
    fn scoping_filters_catalog_probes_to_the_current_schema() {
        let sql = "SELECT 1 FROM information_schema.columns\n    WHERE table_name = 'tasks'; \
                   SELECT 1 FROM pg_constraint WHERE conname = 'x'; \
                   SELECT 1 FROM t WHERE table_names = 'y'";

        assert_eq!(
            scope_to_current_schema(sql),
            "SELECT 1 FROM information_schema.columns\n    WHERE table_schema = current_schema() \
             AND table_name = 'tasks'; SELECT 1 FROM pg_constraint WHERE connamespace = \
             current_schema()::regnamespace AND conname = 'x'; \
             SELECT 1 FROM t WHERE table_names = 'y'",
        );
    }

    /// A catalog probe with no schema filter breaks the isolated E2E run:
    /// write `table_schema = current_schema()`, `connamespace` or a
    /// `::regclass` lookup into it.
    #[test]
    fn every_catalog_probe_names_its_schema() {
        let migrations = sqlx::migrate!();

        for migration in migrations.iter() {
            let scoped = scope_to_current_schema(migration.sql.as_str())
                .lines()
                .map(|line| line.split("--").next().unwrap_or_default())
                .collect::<Vec<_>>()
                .join("\n");
            for catalog in ["information_schema.", "pg_constraint"] {
                for (at, _) in scoped.match_indices(catalog) {
                    let statement = scoped[at..].split(';').next().unwrap_or_default();
                    assert!(
                        [
                            "current_schema()",
                            "table_schema",
                            "connamespace",
                            "::regclass"
                        ]
                        .iter()
                        .any(|filter| statement.contains(filter)),
                        "migration {} probes {catalog} without a schema filter: {}",
                        migration.version,
                        statement.lines().next().unwrap_or_default()
                    );
                }
            }
        }
    }

    /// The constraints a migration checks against every row of a table it did
    /// not create, each as `table.constraint`.
    ///
    /// The scan holds `ACCESS EXCLUSIVE`, stalling the serving replica's reads.
    /// `VALIDATE` in the adding migration still scans under the `ADD`'s lock.
    fn validating_constraints(sql: &str) -> Vec<String> {
        let sql = sql
            .lines()
            .map(|line| line.split("--").next().unwrap_or_default())
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let statements: Vec<Vec<&str>> = sql
            .split(';')
            .map(|statement| statement.split_whitespace().collect())
            .collect();
        let created: Vec<&str> = statements
            .iter()
            .filter_map(|words| match words.as_slice() {
                ["create", "table", "if", "not", "exists", table, ..]
                | ["create", "table", table, ..] => Some(table.trim_end_matches('(')),
                _ => None,
            })
            .collect();
        let mut added = Vec::new();
        let mut validating = Vec::new();
        for words in &statements {
            let ["alter", "table", rest @ ..] = words.as_slice() else {
                continue;
            };
            let rest = rest.strip_prefix(&["if", "exists"]).unwrap_or(rest);
            let rest = rest.strip_prefix(&["only"]).unwrap_or(rest);
            let Some((table, clauses)) = rest.split_first() else {
                continue;
            };
            if created.contains(table) {
                continue;
            }
            let text = format!(" {}", clauses.join(" "));
            for clause in text.split(" add constraint ").skip(1) {
                let name = clause.split_whitespace().next().unwrap_or_default();
                added.push(name.to_owned());
                if (clause.contains(" check") || clause.contains(" foreign key"))
                    && !clause.contains("not valid")
                {
                    validating.push(format!("{table}.{name}"));
                }
            }
            for clause in text.split(" validate constraint ").skip(1) {
                let name = clause.split_whitespace().next().unwrap_or_default();
                if added.iter().any(|added| added == name) {
                    validating.push(format!("{table}.{name}"));
                }
            }
        }
        validating
    }

    #[test]
    fn no_new_migration_validates_a_constraint_under_the_table_lock() {
        let migrations = sqlx::migrate!();

        for migration in migrations.iter() {
            if migration.version <= BASELINE {
                continue;
            }
            let validating = validating_constraints(migration.sql.as_str());
            assert!(
                validating.is_empty(),
                "migration {} adds {validating:?} with a full-table check; add it `NOT VALID` \
                 and `VALIDATE CONSTRAINT` it in a later migration",
                migration.version,
            );
        }
    }

    #[test]
    fn a_rewritten_check_on_an_existing_table_is_flagged() {
        // 0224, which stalled the serving replica's reads for 12 s.
        let pi_harness = "ALTER TABLE trajectory_sessions DROP CONSTRAINT trajectory_sessions_harness_check;\n\
             ALTER TABLE trajectory_sessions ADD CONSTRAINT trajectory_sessions_harness_check CHECK (harness IN ('claude-code', 'codex', 'pi'));\n\
             ALTER TABLE conversation_messages DROP CONSTRAINT conversation_messages_harness_check;\n\
             ALTER TABLE conversation_messages ADD CONSTRAINT conversation_messages_harness_check CHECK (harness IN ('claude-code', 'codex', 'pi'));";

        assert_eq!(
            validating_constraints(pi_harness),
            [
                "trajectory_sessions.trajectory_sessions_harness_check",
                "conversation_messages.conversation_messages_harness_check",
            ],
        );
    }

    #[test]
    fn validating_in_the_adding_migration_is_flagged() {
        let sql = "ALTER TABLE tasks ADD CONSTRAINT tasks_kind_check CHECK (kind IN ('a')) NOT VALID;\n\
                   ALTER TABLE tasks VALIDATE CONSTRAINT tasks_kind_check;";

        assert_eq!(validating_constraints(sql), ["tasks.tasks_kind_check"]);
    }

    #[test]
    fn an_unvalidated_constraint_or_a_new_table_s_own_is_not_flagged() {
        let sql = "ALTER TABLE tasks ADD CONSTRAINT tasks_kind_check CHECK (kind IN ('a')) NOT VALID;\n\
                   CREATE TABLE notes (id int, task_id text);\n\
                   ALTER TABLE notes ADD CONSTRAINT notes_task_fk FOREIGN KEY (task_id) REFERENCES tasks;\n\
                   ALTER TABLE tasks VALIDATE CONSTRAINT tasks_older_check;";

        assert!(validating_constraints(sql).is_empty());
    }
}

#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::*;

    /// CI's `template1` is already migrated, and each test database copies it.
    async fn empty(pool: &DbPool) {
        sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
            .execute(pool)
            .await
            .unwrap();
    }

    async fn versions(pool: &DbPool) -> Vec<i64> {
        sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
            .fetch_all(pool)
            .await
            .unwrap()
    }

    /// Record `rows` of the squashed chain as an old image would have, with no
    /// schema behind them: adoption reads only `_sqlx_migrations`.
    async fn record_squashed(pool: &DbPool, rows: usize) {
        empty(pool).await;
        sqlx::query(
            "CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY, description TEXT NOT NULL, \
             installed_on TIMESTAMPTZ NOT NULL DEFAULT now(), success BOOLEAN NOT NULL, \
             checksum BYTEA NOT NULL, execution_time BIGINT NOT NULL)",
        )
        .execute(pool)
        .await
        .unwrap();
        for (version, description, checksum) in squashed().take(rows) {
            sqlx::query(
                "INSERT INTO _sqlx_migrations \
                 (version, description, success, checksum, execution_time) \
                 VALUES ($1, $2, true, $3, 1)",
            )
            .bind(version)
            .bind(description)
            .bind(checksum)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    fn chain_versions() -> Vec<i64> {
        squashed().map(|(version, ..)| version).collect()
    }

    #[sqlx::test(migrations = false)]
    async fn a_fresh_database_records_only_the_baseline(pool: DbPool) {
        empty(&pool).await;
        run_migrations(&pool).await.unwrap();

        assert_eq!(versions(&pool).await, [BASELINE]);
        let tasks: Option<String> = sqlx::query_scalar("SELECT to_regclass('tasks')::text")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(tasks.as_deref(), Some("tasks"));
    }

    #[sqlx::test(migrations = false)]
    async fn the_whole_chain_is_replaced_by_the_baseline_once(pool: DbPool) {
        record_squashed(&pool, usize::MAX).await;

        run_migrations(&pool).await.unwrap();
        run_migrations(&pool).await.unwrap();

        assert_eq!(versions(&pool).await, [BASELINE]);
    }

    #[sqlx::test(migrations = false)]
    async fn two_replicas_adopting_at_once_both_start(pool: DbPool) {
        record_squashed(&pool, usize::MAX).await;

        let (first, second) = tokio::join!(run_migrations(&pool), run_migrations(&pool));

        first.unwrap();
        second.unwrap();
        assert_eq!(versions(&pool).await, [BASELINE]);
    }

    #[sqlx::test(migrations = false)]
    async fn a_partial_chain_fails_and_is_left_as_it_was(pool: DbPool) {
        record_squashed(&pool, 250).await;

        let error = run_migrations(&pool).await.unwrap_err().to_string();

        assert!(
            error.contains("only part of the migration chain"),
            "{error}"
        );
        assert!(error.contains(&format!("[{}]", BASELINE - 1)), "{error}");
        assert_eq!(versions(&pool).await.len(), 250);
    }

    #[sqlx::test(migrations = false)]
    async fn restoring_returns_the_chain_and_the_next_start_adopts_again(pool: DbPool) {
        record_squashed(&pool, usize::MAX).await;
        run_migrations(&pool).await.unwrap();

        restore_squashed_history(&pool).await.unwrap();
        assert_eq!(versions(&pool).await, chain_versions());
        restore_squashed_history(&pool).await.unwrap();
        assert_eq!(versions(&pool).await, chain_versions());

        run_migrations(&pool).await.unwrap();
        assert_eq!(versions(&pool).await, [BASELINE]);
    }

    #[sqlx::test(migrations = false)]
    async fn restoring_refuses_once_a_later_migration_is_applied(pool: DbPool) {
        empty(&pool).await;
        run_migrations(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
             VALUES ($1, 'later', true, '\\x00', 0)",
        )
        .bind(BASELINE + 1)
        .execute(&pool)
        .await
        .unwrap();

        let error = restore_squashed_history(&pool)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("cannot restore"), "{error}");
        assert_eq!(versions(&pool).await, [BASELINE, BASELINE + 1]);
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "migrations_lock_db_tests.rs"]
mod lock_db_tests;
