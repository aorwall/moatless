//! Workspace database migrations. No other crate declares a migration
//! directory; this is the only one `run_migrations` runs.

use sqlx::AssertSqlSafe;
use sqlx::SqlSafeStr;
use sqlx::migrate::Migrator;

use super::DbPool;

/// Run pending SQL migrations from `./migrations/`. Uses PostgreSQL advisory
/// locks — safe for concurrent replicas.
///
/// Against a schema other than `public` (an isolated E2E run, selected with
/// `search_path` in `DATABASE_URL`) the SQL goes through
/// [`scope_to_current_schema`] first.
pub async fn run_migrations(pool: &DbPool) -> Result<(), sqlx::migrate::MigrateError> {
    let migrator = sqlx::migrate!("./migrations");
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(pool)
        .await?;
    if schema == "public" {
        return migrator.run(pool).await;
    }
    tracing::info!(schema, "Migrating an isolated schema");
    isolated(&migrator).run(pool).await
}

/// `migrator` with every migration's SQL scoped to the current schema.
///
/// Each migration keeps its original checksum, so a schema migrated this way
/// records the same `_sqlx_migrations` rows as `public` does.
fn isolated(migrator: &Migrator) -> Migrator {
    Migrator::with_migrations(
        migrator
            .iter()
            .map(|migration| {
                let mut migration = migration.clone();
                let sql = scope_to_current_schema(migration.sql.as_str());
                migration.sql = AssertSqlSafe(sql).into_sql_str();
                migration
            })
            .collect(),
    )
}

/// Restrict the catalog probes the migrations make to `current_schema()`.
///
/// Unscoped, a probe from a fresh schema finds `public`'s tables and skips the
/// step that would create its own. Fix a probe here, never in an applied
/// migration file: production records its checksum.
fn scope_to_current_schema(sql: &str) -> String {
    let sql = insert_before_column(sql, "table_name", "table_schema = current_schema() AND ");
    let sql = insert_before_column(
        &sql,
        "conname",
        "connamespace = current_schema()::regnamespace AND ",
    );
    // 0090's foreign-key sweep filters on an alias, which the rewrite above
    // does not match.
    sql.replace(
        "WHERE con.contype = 'f' AND ref.relname",
        "WHERE con.contype = 'f' AND con.connamespace = current_schema()::regnamespace \
         AND ref.relname",
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
    use super::scope_to_current_schema;

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

    #[test]
    fn shell_index_migrations_run_outside_a_transaction() {
        let migrations = sqlx::migrate!();

        for version in (107..=110).chain(209..=210).chain([219]) {
            let migration = migrations
                .iter()
                .find(|migration| migration.version == version)
                .expect("shell index migration");
            assert!(migration.no_tx, "migration {version}");
        }
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
}
