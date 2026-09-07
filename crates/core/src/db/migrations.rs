//! Workspace database migrations.

use super::DbPool;

/// Run pending SQL migrations from `./migrations/`.
/// Uses PostgreSQL advisory locks — safe for concurrent replicas.
pub async fn run_migrations(pool: &DbPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}

#[cfg(test)]
mod tests {
    /// Two files sharing a version is not a conflict git can see: both merge,
    /// and the loser is silently skipped against a database that recorded the
    /// winner, leaving its schema change unapplied and startup refusing to boot.
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

        for version in 107..=110 {
            let migration = migrations
                .iter()
                .find(|migration| migration.version == version)
                .expect("shell index migration");
            assert!(migration.no_tx, "migration {version}");
        }
    }
}
