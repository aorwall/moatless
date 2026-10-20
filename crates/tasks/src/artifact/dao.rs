//! Artifact DAO — queries for `task_artifacts` and `artifact_records`.

use async_trait::async_trait;
use sqlx::PgConnection;
use uuid::Uuid;

use super::models::{Artifact, ArtifactRecord, DataAccess, Reservation};
use crate::conversation::projector::Projector;
use crate::task_event::models::InsertTaskEvent;
use moatless_core::db::DbPool;
use moatless_types::activity::ArtifactShown;
use moatless_types::event::EventType;
use moatless_types::task::TaskSubType;

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
    /// when no row carries `artifact_id`. An older version leaves
    /// `latest_version`, `title` and `data` alone. `shown` also adds it to
    /// `inline_versions` and stores its `artifact_shown` event in one
    /// transaction.
    async fn commit_version(
        &self,
        artifact_id: &str,
        version: i32,
        title: &str,
        data: DataAccess,
        shown: Option<ArtifactShown>,
    ) -> Result<Option<Artifact>, sqlx::Error>;
    /// Find an Artifact by id, published or not.
    async fn find_by_id(&self, artifact_id: &str) -> Result<Option<Artifact>, sqlx::Error>;
    /// The Task's published Artifacts, most recently updated first.
    async fn list_for_task(&self, task_id: &str) -> Result<Vec<Artifact>, sqlx::Error>;
    /// Append a record as `record_id`, or with a `key`, replace the record
    /// saved under it, keeping that record's id and `created_at`.
    async fn save_record(
        &self,
        record_id: Uuid,
        artifact_id: &str,
        key: Option<String>,
        data: &serde_json::Value,
        submitted_by: &str,
    ) -> Result<ArtifactRecord, sqlx::Error>;
    /// How many records the Artifact holds, and whether one is saved under
    /// `key`.
    async fn count_records(
        &self,
        artifact_id: &str,
        key: Option<String>,
    ) -> Result<(i64, bool), sqlx::Error>;
    /// The Artifact's records changed after `since`, oldest change first.
    async fn list_records(
        &self,
        artifact_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<ArtifactRecord>, sqlx::Error>;
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
        data: DataAccess,
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
        record_id: Uuid,
        artifact_id: &str,
        key: Option<String>,
        data: &serde_json::Value,
        submitted_by: &str,
    ) -> Result<ArtifactRecord, sqlx::Error> {
        sqlx::query_as::<_, ArtifactRecord>(
            r#"INSERT INTO artifact_records (record_id, artifact_id, key, data, submitted_by)
               VALUES ($1, $2, $3, $4, $5)
               ON CONFLICT (artifact_id, key) WHERE key IS NOT NULL
               DO UPDATE SET data = EXCLUDED.data,
                             submitted_by = EXCLUDED.submitted_by,
                             updated_at = now()
               RETURNING *"#,
        )
        .bind(record_id)
        .bind(artifact_id)
        .bind(key)
        .bind(data)
        .bind(submitted_by)
        .fetch_one(&self.0)
        .await
    }

    async fn count_records(
        &self,
        artifact_id: &str,
        key: Option<String>,
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

    async fn list_records(
        &self,
        artifact_id: &str,
        since: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<Vec<ArtifactRecord>, sqlx::Error> {
        sqlx::query_as::<_, ArtifactRecord>(
            r#"SELECT * FROM artifact_records
               WHERE artifact_id = $1 AND ($2::timestamptz IS NULL OR updated_at > $2)
               ORDER BY updated_at, record_id"#,
        )
        .bind(artifact_id)
        .bind(since)
        .fetch_all(&self.0)
        .await
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
