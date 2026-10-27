//! Artifact DAO — queries for `task_artifacts` and `artifact_records`.

use async_trait::async_trait;
use sqlx::PgConnection;
use uuid::Uuid;

use super::models::{Artifact, ArtifactRecord, DataAccess, Reservation};
use crate::conversation::projector::Projector;
use crate::task_event::models::InsertTaskEvent;
use moatless_api_schemas::artifact::RecordWriter;
use moatless_core::db::DbPool;
use moatless_types::activity::ArtifactShown;
use moatless_types::event::EventType;
use moatless_types::task::{TaskSubType, Visibility};

/// DAO trait for Artifact persistence. Callers check Task access first:
/// `task_artifacts` carries no scope columns of its own.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ArtifactDao: Send + Sync {
    /// Reserve the next version of `(task_id, slug)`, creating the row as
    /// `new_artifact_id` when the slug is new. Two concurrent publishes to one
    /// slug reserve different versions.
    async fn reserve_version(
        &self,
        new_artifact_id: &str,
        task_id: &str,
        slug: &str,
        title: &str,
        created_by: &str,
    ) -> Result<Reservation, sqlx::Error>;
    /// Add `version` to `published_versions` and return the row, or `None`
    /// when no row carries `artifact_id`. Only a newer version sets the other
    /// arguments, and a `None` visibility keeps the row's. `shown` also adds it
    /// to `inline_versions` and stores its `artifact_shown` event in one
    /// transaction.
    async fn commit_version(
        &self,
        artifact_id: &str,
        version: i32,
        title: &str,
        data: &DataAccess,
        visibility: Option<Visibility>,
        shown: Option<ArtifactShown>,
    ) -> Result<Option<Artifact>, sqlx::Error>;
    /// Set a published Artifact's visibility and return the row, or `None` when
    /// no published row carries `artifact_id`. Leaves `updated_at` alone: it
    /// orders the list by when a page last changed.
    async fn set_visibility(
        &self,
        artifact_id: &str,
        visibility: Visibility,
    ) -> Result<Option<Artifact>, sqlx::Error>;
    /// Find an Artifact by id, published or not.
    async fn find_by_id(&self, artifact_id: &str) -> Result<Option<Artifact>, sqlx::Error>;
    /// The Task's published Artifacts, most recently updated first.
    async fn list_for_task(&self, task_id: &str) -> Result<Vec<Artifact>, sqlx::Error>;
    /// Save `data` under `key`, replacing the record already there but
    /// keeping its `created_at`.
    async fn save_record(
        &self,
        artifact_id: &str,
        key: &str,
        data: &serde_json::Value,
        submitted_by: &str,
        writer: RecordWriter,
    ) -> Result<ArtifactRecord, sqlx::Error>;
    /// Save `data` under `key` only when no record is there, answering `None`
    /// when one is.
    async fn insert_record(
        &self,
        artifact_id: &str,
        key: &str,
        data: &serde_json::Value,
        submitted_by: &str,
        writer: RecordWriter,
    ) -> Result<Option<ArtifactRecord>, sqlx::Error>;
    /// Replace the data of the record under `key` while it is still as it was
    /// at `stored_at`, its `updated_at`, keeping its `submitted_by` and
    /// `writer`. Answers `None` when the record changed or is gone.
    async fn update_record(
        &self,
        artifact_id: &str,
        key: &str,
        data: &serde_json::Value,
        stored_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<ArtifactRecord>, sqlx::Error>;
    /// Remove the record under `key` while it is still as it was at
    /// `stored_at`, returning it as it was; `None` when it changed or is gone.
    async fn delete_record_at(
        &self,
        artifact_id: &str,
        key: &str,
        stored_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<ArtifactRecord>, sqlx::Error>;
    /// The record under `key`, if any.
    async fn find_record(
        &self,
        artifact_id: &str,
        key: &str,
    ) -> Result<Option<ArtifactRecord>, sqlx::Error>;
    /// How many records the Artifact holds, and whether one is saved under
    /// `key`.
    async fn count_records(&self, artifact_id: &str, key: &str)
    -> Result<(i64, bool), sqlx::Error>;
    /// Remove the record under `key`, returning it as it was.
    async fn delete_record(
        &self,
        artifact_id: &str,
        key: &str,
    ) -> Result<Option<ArtifactRecord>, sqlx::Error>;
    /// The Artifact's records changed after `since` whose keys start with
    /// `prefix`, each when given, oldest change first.
    async fn list_records(
        &self,
        artifact_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
        prefix: Option<String>,
    ) -> Result<Vec<ArtifactRecord>, sqlx::Error>;
    /// The records under `keys`; a key with no record is left out.
    async fn find_records(
        &self,
        artifact_id: &str,
        keys: &[String],
    ) -> Result<Vec<ArtifactRecord>, sqlx::Error>;
    /// Each listed Artifact's `record_seq`, the count of record writes so far.
    async fn record_heads(
        &self,
        artifact_ids: &[String],
    ) -> Result<Vec<(String, i64)>, sqlx::Error>;
    /// The Artifact's `record_seq`, and the `(seq, key)` of each logged change
    /// after `after` up to it, oldest first. A change pruned from the log is
    /// missing, so the first `seq` is past `after + 1`.
    async fn record_changes(
        &self,
        artifact_id: &str,
        after: i64,
    ) -> Result<(i64, Vec<(i64, String)>), sqlx::Error>;
}

/// How many changes per Artifact `artifact_record_changes` keeps.
const KEPT_CHANGES: i64 = 1000;

/// Log a record write to `key`. Call it first in the write's transaction: the
/// row lock it takes on `task_artifacts` orders `seq` by commit, and taking it
/// before the record's lock matches the order a cascading delete takes them.
/// A write that changes no record drops its transaction, so it logs nothing.
async fn log_change(
    conn: &mut PgConnection,
    artifact_id: &str,
    key: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"WITH bumped AS (
               UPDATE task_artifacts SET record_seq = record_seq + 1
               WHERE artifact_id = $1 RETURNING record_seq
           ), logged AS (
               INSERT INTO artifact_record_changes (artifact_id, seq, key)
               SELECT $1, record_seq, $2 FROM bumped RETURNING seq
           )
           DELETE FROM artifact_record_changes
           WHERE artifact_id = $1 AND seq <= (SELECT seq FROM logged) - $3"#,
    )
    .bind(artifact_id)
    .bind(key)
    .bind(KEPT_CHANGES)
    .execute(conn)
    .await?;
    Ok(())
}

/// PostgreSQL implementation of [`ArtifactDao`].
pub struct PgArtifactDao(pub DbPool);

#[async_trait]
impl ArtifactDao for PgArtifactDao {
    async fn reserve_version(
        &self,
        new_artifact_id: &str,
        task_id: &str,
        slug: &str,
        title: &str,
        created_by: &str,
    ) -> Result<Reservation, sqlx::Error> {
        sqlx::query_as::<_, Reservation>(
            r#"INSERT INTO task_artifacts (artifact_id, task_id, slug, title, created_by)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (task_id, slug)
               DO UPDATE SET reserved_version = task_artifacts.reserved_version + 1
               RETURNING artifact_id, reserved_version"#,
        )
        .bind(new_artifact_id)
        .bind(task_id)
        .bind(slug)
        .bind(title)
        .bind(created_by)
        .fetch_one(&self.0)
        .await
    }

    async fn commit_version(
        &self,
        artifact_id: &str,
        version: i32,
        title: &str,
        data: &DataAccess,
        visibility: Option<Visibility>,
        shown: Option<ArtifactShown>,
    ) -> Result<Option<Artifact>, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        let row = sqlx::query_as::<_, Artifact>(
            r#"UPDATE task_artifacts SET
                   title = CASE WHEN $2 > latest_version THEN $3 ELSE title END,
                   latest_version = GREATEST(latest_version, $2),
                   published_versions = array_append(published_versions, $2),
                   inline_versions = CASE WHEN $4 THEN array_append(inline_versions, $2)
                                          ELSE inline_versions END,
                   accepts_data = CASE WHEN $2 > latest_version THEN $5 ELSE accepts_data END,
                   notify_on_data = CASE WHEN $2 > latest_version THEN $6
                                         ELSE notify_on_data END,
                   visibility = CASE WHEN $2 > latest_version THEN COALESCE($7, visibility)
                                     ELSE visibility END,
                   record_rules = CASE WHEN $2 > latest_version THEN $8
                                       ELSE record_rules END,
                   updated_at = now()
               WHERE artifact_id = $1
               RETURNING *"#,
        )
        .bind(artifact_id)
        .bind(version)
        .bind(title)
        .bind(shown.is_some())
        .bind(data.accepts_data)
        .bind(data.notify_on_data)
        .bind(visibility.map(Visibility::as_str))
        .bind(data.rules.as_ref().map(sqlx::types::Json))
        .fetch_optional(&mut *tx)
        .await?;
        if let (Some(row), Some(shown)) = (&row, &shown) {
            let event = shown_event(&mut tx, &row.task_id, shown).await?;
            crate::task_event::dao::insert_in(&mut tx, &event).await?;
            Projector.on_task_event(&mut tx, &event).await?;
        }
        tx.commit().await?;
        Ok(row)
    }

    async fn set_visibility(
        &self,
        artifact_id: &str,
        visibility: Visibility,
    ) -> Result<Option<Artifact>, sqlx::Error> {
        sqlx::query_as::<_, Artifact>(
            r#"UPDATE task_artifacts SET visibility = $2
               WHERE artifact_id = $1 AND latest_version > 0
               RETURNING *"#,
        )
        .bind(artifact_id)
        .bind(visibility.as_str())
        .fetch_optional(&self.0)
        .await
    }

    async fn find_by_id(&self, artifact_id: &str) -> Result<Option<Artifact>, sqlx::Error> {
        sqlx::query_as::<_, Artifact>("SELECT * FROM task_artifacts WHERE artifact_id = $1")
            .bind(artifact_id)
            .fetch_optional(&self.0)
            .await
    }

    async fn list_for_task(&self, task_id: &str) -> Result<Vec<Artifact>, sqlx::Error> {
        sqlx::query_as::<_, Artifact>(
            r#"SELECT * FROM task_artifacts
               WHERE task_id = $1 AND latest_version > 0
               ORDER BY updated_at DESC, artifact_id"#,
        )
        .bind(task_id)
        .fetch_all(&self.0)
        .await
    }

    async fn save_record(
        &self,
        artifact_id: &str,
        key: &str,
        data: &serde_json::Value,
        submitted_by: &str,
        writer: RecordWriter,
    ) -> Result<ArtifactRecord, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        log_change(&mut tx, artifact_id, key).await?;
        let record = sqlx::query_as::<_, ArtifactRecord>(
            r#"INSERT INTO artifact_records (artifact_id, key, data, submitted_by, writer)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (artifact_id, key)
               DO UPDATE SET data = EXCLUDED.data,
                             submitted_by = EXCLUDED.submitted_by,
                             writer = EXCLUDED.writer,
                             updated_at = now()
               RETURNING *"#,
        )
        .bind(artifact_id)
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
        artifact_id: &str,
        key: &str,
        data: &serde_json::Value,
        submitted_by: &str,
        writer: RecordWriter,
    ) -> Result<Option<ArtifactRecord>, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        log_change(&mut tx, artifact_id, key).await?;
        let record = sqlx::query_as::<_, ArtifactRecord>(
            r#"INSERT INTO artifact_records (artifact_id, key, data, submitted_by, writer)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (artifact_id, key) DO NOTHING
               RETURNING *"#,
        )
        .bind(artifact_id)
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
        artifact_id: &str,
        key: &str,
        data: &serde_json::Value,
        stored_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<ArtifactRecord>, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        log_change(&mut tx, artifact_id, key).await?;
        // Strictly later than the `updated_at` it replaces, so two updates
        // whose transactions started in the same microsecond cannot both
        // match one `stored_at`.
        let record = sqlx::query_as::<_, ArtifactRecord>(
            r#"UPDATE artifact_records
               SET data = $3,
                   updated_at = GREATEST(clock_timestamp(), updated_at + interval '1 microsecond')
               WHERE artifact_id = $1 AND key = $2 AND updated_at = $4
               RETURNING *"#,
        )
        .bind(artifact_id)
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
        artifact_id: &str,
        key: &str,
        stored_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Option<ArtifactRecord>, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        log_change(&mut tx, artifact_id, key).await?;
        let record = sqlx::query_as::<_, ArtifactRecord>(
            r#"DELETE FROM artifact_records
               WHERE artifact_id = $1 AND key = $2 AND updated_at = $3
               RETURNING *"#,
        )
        .bind(artifact_id)
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
        artifact_id: &str,
        key: &str,
    ) -> Result<Option<ArtifactRecord>, sqlx::Error> {
        sqlx::query_as::<_, ArtifactRecord>(
            "SELECT * FROM artifact_records WHERE artifact_id = $1 AND key = $2",
        )
        .bind(artifact_id)
        .bind(key)
        .fetch_optional(&self.0)
        .await
    }

    async fn count_records(
        &self,
        artifact_id: &str,
        key: &str,
    ) -> Result<(i64, bool), sqlx::Error> {
        sqlx::query_as::<_, (i64, bool)>(
            r#"SELECT count(*), COALESCE(bool_or(key = $2), false)
               FROM artifact_records WHERE artifact_id = $1"#,
        )
        .bind(artifact_id)
        .bind(key)
        .fetch_one(&self.0)
        .await
    }

    async fn delete_record(
        &self,
        artifact_id: &str,
        key: &str,
    ) -> Result<Option<ArtifactRecord>, sqlx::Error> {
        let mut tx = self.0.begin().await?;
        log_change(&mut tx, artifact_id, key).await?;
        let record = sqlx::query_as::<_, ArtifactRecord>(
            "DELETE FROM artifact_records WHERE artifact_id = $1 AND key = $2 RETURNING *",
        )
        .bind(artifact_id)
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
        artifact_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
        prefix: Option<String>,
    ) -> Result<Vec<ArtifactRecord>, sqlx::Error> {
        sqlx::query_as::<_, ArtifactRecord>(
            r#"SELECT * FROM artifact_records
               WHERE artifact_id = $1 AND ($2::timestamptz IS NULL OR updated_at > $2)
                 AND ($3::text IS NULL OR starts_with(key, $3))
               ORDER BY updated_at, key"#,
        )
        .bind(artifact_id)
        .bind(since)
        .bind(prefix)
        .fetch_all(&self.0)
        .await
    }

    async fn find_records(
        &self,
        artifact_id: &str,
        keys: &[String],
    ) -> Result<Vec<ArtifactRecord>, sqlx::Error> {
        sqlx::query_as::<_, ArtifactRecord>(
            "SELECT * FROM artifact_records WHERE artifact_id = $1 AND key = ANY($2)",
        )
        .bind(artifact_id)
        .bind(keys)
        .fetch_all(&self.0)
        .await
    }

    async fn record_heads(
        &self,
        artifact_ids: &[String],
    ) -> Result<Vec<(String, i64)>, sqlx::Error> {
        sqlx::query_as::<_, (String, i64)>(
            "SELECT artifact_id, record_seq FROM task_artifacts WHERE artifact_id = ANY($1)",
        )
        .bind(artifact_ids)
        .fetch_all(&self.0)
        .await
    }

    async fn record_changes(
        &self,
        artifact_id: &str,
        after: i64,
    ) -> Result<(i64, Vec<(i64, String)>), sqlx::Error> {
        let head: Option<i64> =
            sqlx::query_scalar("SELECT record_seq FROM task_artifacts WHERE artifact_id = $1")
                .bind(artifact_id)
                .fetch_optional(&self.0)
                .await?;
        let head = head.unwrap_or_default();
        // Bounded by `head` so the caller's gap check compares a head and a
        // log read as of the same write.
        let changes = sqlx::query_as::<_, (i64, String)>(
            r#"SELECT seq, key FROM artifact_record_changes
               WHERE artifact_id = $1 AND seq > $2 AND seq <= $3
               ORDER BY seq"#,
        )
        .bind(artifact_id)
        .bind(after)
        .bind(head)
        .fetch_all(&self.0)
        .await?;
        Ok((head, changes))
    }
}

/// The `artifact_shown` event for `shown`, in the Task's latest turn. Its id is
/// derived from the version, so a version is shown once.
async fn shown_event(
    tx: &mut PgConnection,
    task_id: &str,
    shown: &ArtifactShown,
) -> Result<InsertTaskEvent, sqlx::Error> {
    let turn_number: Option<i32> =
        sqlx::query_scalar("SELECT MAX(turn_number) FROM task_turns WHERE task_id = $1")
            .bind(task_id)
            .fetch_one(&mut *tx)
            .await?;
    let name = format!("artifact_shown:{}:v{}", shown.artifact_id, shown.version);
    Ok(InsertTaskEvent {
        event_id: Uuid::new_v5(&Uuid::NAMESPACE_OID, name.as_bytes()).to_string(),
        task_id: task_id.to_string(),
        event_type: EventType::Task.as_str().to_string(),
        sub_type: TaskSubType::ArtifactShown.as_str().to_string(),
        is_error: false,
        agent_profile: None,
        turn_number,
        payload: serde_json::to_value(shown).unwrap_or_default(),
        timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    })
}

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod dao_db_tests;
