//! Record DAO — queries for `record_collections`, `collection_records` and
//! `collection_record_changes`.

use async_trait::async_trait;
use sqlx::PgConnection;

use super::models::{CollectionRow, Record};
use moatless_api_schemas::artifact::RecordWriter;
use moatless_core::db::DbPool;

/// DAO trait for record collections and their records. Callers check access
/// first: a collection carries its scope, not who may read it.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait RecordDao: Send + Sync {
    /// The collection `(scope_kind, scope_id, name)`, creating it as
    /// `new_collection_id` when it does not exist. Never an `artifact` one:
    /// those are made with their Artifact.
    async fn ensure_collection(
        &self,
        new_collection_id: &str,
        scope_kind: &str,
        scope_id: &str,
        name: &str,
        created_by_task_id: &str,
        created_by_user_id: &str,
    ) -> Result<CollectionRow, sqlx::Error>;
    /// The collection `(scope_kind, scope_id, name)`, if it exists.
    async fn find_collection(
        &self,
        scope_kind: &str,
        scope_id: &str,
        name: &str,
    ) -> Result<Option<CollectionRow>, sqlx::Error>;
    /// The scope's collections by name.
    async fn list_collections(
        &self,
        scope_kind: &str,
        scope_id: &str,
    ) -> Result<Vec<CollectionRow>, sqlx::Error>;
    /// Save `data` under `key`, replacing the record already there but
    /// keeping its `created_at`.
    async fn save_record(
        &self,
        collection_id: &str,
        key: &str,
        data: &serde_json::Value,
        submitted_by: &str,
        writer: RecordWriter,
    ) -> Result<Record, sqlx::Error>;
    /// Save `data` under `key` only when no record is there, answering `None`
    /// when one is.
    async fn insert_record(
        &self,
        collection_id: &str,
        key: &str,
        data: &serde_json::Value,
        submitted_by: &str,
        writer: RecordWriter,
    ) -> Result<Option<Record>, sqlx::Error>;
    /// Replace the data of the record under `key` while it is still as it was
    /// at `stored_at`, its `updated_at`, keeping its `submitted_by` and
    /// `writer`. Answers `None` when the record changed or is gone.
    async fn update_record(
        &self,
        collection_id: &str,
        key: &str,
        data: &serde_json::Value,
        stored_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<Record>, sqlx::Error>;
    /// Remove the record under `key` while it is still as it was at
    /// `stored_at`, returning it as it was; `None` when it changed or is gone.
    async fn delete_record_at(
        &self,
        collection_id: &str,
        key: &str,
        stored_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<Record>, sqlx::Error>;
    /// The record under `key`, if any.
    async fn find_record(
        &self,
        collection_id: &str,
        key: &str,
    ) -> Result<Option<Record>, sqlx::Error>;
    /// How many records the collection holds, and whether one is saved under
    /// `key`.
    async fn count_records(
        &self,
        collection_id: &str,
        key: &str,
    ) -> Result<(i64, bool), sqlx::Error>;
    /// Remove the record under `key`, returning it as it was.
    async fn delete_record(
        &self,
        collection_id: &str,
        key: &str,
    ) -> Result<Option<Record>, sqlx::Error>;
    /// The collection's records changed after `since` whose keys start with
    /// `prefix`, each when given, oldest change first.
    async fn list_records(
        &self,
        collection_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
        prefix: Option<String>,
    ) -> Result<Vec<Record>, sqlx::Error>;
    /// The records under `keys`; a key with no record is left out.
    async fn find_records(
        &self,
        collection_id: &str,
        keys: &[String],
    ) -> Result<Vec<Record>, sqlx::Error>;
    /// Each listed collection's `record_seq`, the count of record writes so
    /// far.
    async fn record_heads(
        &self,
        collection_ids: &[String],
    ) -> Result<Vec<(String, i64)>, sqlx::Error>;
    /// The collection's `record_seq`, and the `(seq, key)` of each logged
    /// change after `after` up to it, oldest first. A change pruned from the
    /// log is missing, so the first `seq` is past `after + 1`.
    async fn record_changes(
        &self,
        collection_id: &str,
        after: i64,
    ) -> Result<(i64, Vec<(i64, String)>), sqlx::Error>;
}

/// How many changes per collection `collection_record_changes` keeps.
const KEPT_CHANGES: i64 = 1000;

/// The columns of a [`CollectionRow`], over `record_collections c`.
const COLLECTION_COLUMNS: &str = r#"c.collection_id, c.scope_kind, c.scope_id, c.name, c.created_at,
       (SELECT count(*) FROM collection_records r WHERE r.collection_id = c.collection_id)
           AS record_count,
       (SELECT max(updated_at) FROM collection_records r WHERE r.collection_id = c.collection_id)
           AS last_updated_at"#;

/// Log a record write to `key`. Call it first in the write's transaction: the
/// row lock it takes on `record_collections` orders `seq` by commit, and
/// taking it before the record's lock matches the order a cascading delete
/// takes them. A write that changes no record drops its transaction, so it
/// logs nothing.
async fn log_change(
    conn: &mut PgConnection,
    collection_id: &str,
    key: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"WITH bumped AS (
               UPDATE record_collections SET record_seq = record_seq + 1
               WHERE collection_id = $1 RETURNING record_seq
           ), logged AS (
               INSERT INTO collection_record_changes (collection_id, seq, key)
               SELECT $1, record_seq, $2 FROM bumped RETURNING seq
           )
           DELETE FROM collection_record_changes
           WHERE collection_id = $1 AND seq <= (SELECT seq FROM logged) - $3"#,
    )
    .bind(collection_id)
    .bind(key)
    .bind(KEPT_CHANGES)
    .execute(conn)
    .await?;
    Ok(())
}

/// PostgreSQL implementation of [`RecordDao`].
pub struct PgRecordDao(pub DbPool);

#[async_trait]
impl RecordDao for PgRecordDao {
    async fn ensure_collection(
        &self,
        new_collection_id: &str,
        scope_kind: &str,
        scope_id: &str,
        name: &str,
        created_by_task_id: &str,
        created_by_user_id: &str,
    ) -> Result<CollectionRow, sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO record_collections
                   (collection_id, scope_kind, scope_id, name, created_by_task_id, created_by_user_id)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT (scope_kind, scope_id, name) DO NOTHING"#,
        )
        .bind(new_collection_id)
        .bind(scope_kind)
        .bind(scope_id)
        .bind(name)
        .bind(created_by_task_id)
        .bind(created_by_user_id)
        .execute(&self.0)
        .await?;
        self.find_collection(scope_kind, scope_id, name)
            .await?
            .ok_or(sqlx::Error::RowNotFound)
    }

    async fn find_collection(
        &self,
        scope_kind: &str,
        scope_id: &str,
        name: &str,
    ) -> Result<Option<CollectionRow>, sqlx::Error> {
        let sql = format!(
            "SELECT {COLLECTION_COLUMNS} FROM record_collections c \
             WHERE c.scope_kind = $1 AND c.scope_id = $2 AND c.name = $3"
        );
        sqlx::query_as::<_, CollectionRow>(sqlx::AssertSqlSafe(sql))
            .bind(scope_kind)
            .bind(scope_id)
            .bind(name)
            .fetch_optional(&self.0)
            .await
    }

    async fn list_collections(
        &self,
        scope_kind: &str,
        scope_id: &str,
    ) -> Result<Vec<CollectionRow>, sqlx::Error> {
        let sql = format!(
            "SELECT {COLLECTION_COLUMNS} FROM record_collections c \
             WHERE c.scope_kind = $1 AND c.scope_id = $2 ORDER BY c.name"
        );
        sqlx::query_as::<_, CollectionRow>(sqlx::AssertSqlSafe(sql))
            .bind(scope_kind)
            .bind(scope_id)
            .fetch_all(&self.0)
            .await
    }

    async fn save_record(
        &self,
        collection_id: &str,
        key: &str,
        data: &serde_json::Value,
        submitted_by: &str,
        writer: RecordWriter,
    ) -> Result<Record, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        log_change(&mut tx, collection_id, key).await?;
        let record = sqlx::query_as::<_, Record>(
            r#"INSERT INTO collection_records (collection_id, key, data, submitted_by, writer)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (collection_id, key)
               DO UPDATE SET data = EXCLUDED.data,
                             submitted_by = EXCLUDED.submitted_by,
                             writer = EXCLUDED.writer,
                             updated_at = now()
               RETURNING *"#,
        )
        .bind(collection_id)
        .bind(key)
        .bind(data)
        .bind(submitted_by)
        .bind(writer.as_str())
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(record)
    }

    async fn insert_record(
        &self,
        collection_id: &str,
        key: &str,
        data: &serde_json::Value,
        submitted_by: &str,
        writer: RecordWriter,
    ) -> Result<Option<Record>, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        log_change(&mut tx, collection_id, key).await?;
        let record = sqlx::query_as::<_, Record>(
            r#"INSERT INTO collection_records (collection_id, key, data, submitted_by, writer)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (collection_id, key) DO NOTHING
               RETURNING *"#,
        )
        .bind(collection_id)
        .bind(key)
        .bind(data)
        .bind(submitted_by)
        .bind(writer.as_str())
        .fetch_optional(&mut *tx)
        .await?;
        if record.is_some() {
            tx.commit().await?;
        }
        Ok(record)
    }

    async fn update_record(
        &self,
        collection_id: &str,
        key: &str,
        data: &serde_json::Value,
        stored_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<Record>, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        log_change(&mut tx, collection_id, key).await?;
        // Strictly later than the `updated_at` it replaces, so two updates
        // whose transactions started in the same microsecond cannot both
        // match one `stored_at`.
        let record = sqlx::query_as::<_, Record>(
            r#"UPDATE collection_records
               SET data = $3,
                   updated_at = GREATEST(clock_timestamp(), updated_at + interval '1 microsecond')
               WHERE collection_id = $1 AND key = $2 AND updated_at = $4
               RETURNING *"#,
        )
        .bind(collection_id)
        .bind(key)
        .bind(data)
        .bind(stored_at)
        .fetch_optional(&mut *tx)
        .await?;
        if record.is_some() {
            tx.commit().await?;
        }
        Ok(record)
    }

    async fn delete_record_at(
        &self,
        collection_id: &str,
        key: &str,
        stored_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<Record>, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        log_change(&mut tx, collection_id, key).await?;
        let record = sqlx::query_as::<_, Record>(
            r#"DELETE FROM collection_records
               WHERE collection_id = $1 AND key = $2 AND updated_at = $3
               RETURNING *"#,
        )
        .bind(collection_id)
        .bind(key)
        .bind(stored_at)
        .fetch_optional(&mut *tx)
        .await?;
        if record.is_some() {
            tx.commit().await?;
        }
        Ok(record)
    }

    async fn find_record(
        &self,
        collection_id: &str,
        key: &str,
    ) -> Result<Option<Record>, sqlx::Error> {
        sqlx::query_as::<_, Record>(
            "SELECT * FROM collection_records WHERE collection_id = $1 AND key = $2",
        )
        .bind(collection_id)
        .bind(key)
        .fetch_optional(&self.0)
        .await
    }

    async fn count_records(
        &self,
        collection_id: &str,
        key: &str,
    ) -> Result<(i64, bool), sqlx::Error> {
        sqlx::query_as::<_, (i64, bool)>(
            r#"SELECT count(*), COALESCE(bool_or(key = $2), false)
               FROM collection_records WHERE collection_id = $1"#,
        )
        .bind(collection_id)
        .bind(key)
        .fetch_one(&self.0)
        .await
    }

    async fn delete_record(
        &self,
        collection_id: &str,
        key: &str,
    ) -> Result<Option<Record>, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        log_change(&mut tx, collection_id, key).await?;
        let record = sqlx::query_as::<_, Record>(
            "DELETE FROM collection_records WHERE collection_id = $1 AND key = $2 RETURNING *",
        )
        .bind(collection_id)
        .bind(key)
        .fetch_optional(&mut *tx)
        .await?;
        if record.is_some() {
            tx.commit().await?;
        }
        Ok(record)
    }

    async fn list_records(
        &self,
        collection_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
        prefix: Option<String>,
    ) -> Result<Vec<Record>, sqlx::Error> {
        sqlx::query_as::<_, Record>(
            r#"SELECT * FROM collection_records
               WHERE collection_id = $1 AND ($2::timestamptz IS NULL OR updated_at > $2)
                 AND ($3::text IS NULL OR starts_with(key, $3))
               ORDER BY updated_at, key"#,
        )
        .bind(collection_id)
        .bind(since)
        .bind(prefix)
        .fetch_all(&self.0)
        .await
    }

    async fn find_records(
        &self,
        collection_id: &str,
        keys: &[String],
    ) -> Result<Vec<Record>, sqlx::Error> {
        sqlx::query_as::<_, Record>(
            "SELECT * FROM collection_records WHERE collection_id = $1 AND key = ANY($2)",
        )
        .bind(collection_id)
        .bind(keys)
        .fetch_all(&self.0)
        .await
    }

    async fn record_heads(
        &self,
        collection_ids: &[String],
    ) -> Result<Vec<(String, i64)>, sqlx::Error> {
        sqlx::query_as::<_, (String, i64)>(
            "SELECT collection_id, record_seq FROM record_collections WHERE collection_id = ANY($1)",
        )
        .bind(collection_ids)
        .fetch_all(&self.0)
        .await
    }

    async fn record_changes(
        &self,
        collection_id: &str,
        after: i64,
    ) -> Result<(i64, Vec<(i64, String)>), sqlx::Error> {
        let head: Option<i64> = sqlx::query_scalar(
            "SELECT record_seq FROM record_collections WHERE collection_id = $1",
        )
        .bind(collection_id)
        .fetch_optional(&self.0)
        .await?;
        let head = head.unwrap_or_default();
        // Bounded by `head` so the caller's gap check compares a head and a
        // log read as of the same write.
        let changes = sqlx::query_as::<_, (i64, String)>(
            r#"SELECT seq, key FROM collection_record_changes
               WHERE collection_id = $1 AND seq > $2 AND seq <= $3
               ORDER BY seq"#,
        )
        .bind(collection_id)
        .bind(after)
        .bind(head)
        .fetch_all(&self.0)
        .await?;
        Ok((head, changes))
    }
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod dao_db_tests;
