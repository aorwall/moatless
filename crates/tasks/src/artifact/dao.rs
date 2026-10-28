//! Artifact DAO — queries for `task_artifacts`.

use async_trait::async_trait;
use sqlx::PgConnection;
use uuid::Uuid;

use super::models::{Artifact, DataAccess, Reservation};
use crate::conversation::projector::Projector;
use crate::task_event::models::InsertTaskEvent;
use moatless_core::db::DbPool;
use moatless_types::activity::ArtifactShown;
use moatless_types::event::EventType;
use moatless_types::task::{TaskSubType, Visibility};

/// DAO trait for Artifact persistence. Callers check Task access first:
/// `task_artifacts` carries no scope columns of its own.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait ArtifactDao: Send + Sync {
    /// Reserve the next version of `(task_id, slug)`, creating the row and its
    /// own record collection as `new_artifact_id` when the slug is new. Two
    /// concurrent publishes to one slug reserve different versions.
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
        // The collection row references the Artifact row this statement
        // inserts; a foreign key is checked at the end of the statement, so
        // the one statement satisfies it.
        sqlx::query_as::<_, Reservation>(
            r#"WITH reserved AS (
                   INSERT INTO task_artifacts (artifact_id, task_id, slug, title, created_by)
                   VALUES ($1, $2, $3, $4, $5)
                   ON CONFLICT (task_id, slug)
                   DO UPDATE SET reserved_version = task_artifacts.reserved_version + 1
                   RETURNING artifact_id, reserved_version
               ), collection AS (
                   INSERT INTO record_collections
                       (collection_id, scope_kind, scope_id, name, artifact_id, created_by_task_id,
                        created_by_user_id)
                   SELECT artifact_id, 'artifact', artifact_id, '', artifact_id, $2, $5
                   FROM reserved
                   ON CONFLICT DO NOTHING
               )
               SELECT artifact_id, reserved_version FROM reserved"#,
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
                   collections = CASE WHEN $2 > latest_version THEN $9
                                      ELSE collections END,
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
        .bind(sqlx::types::Json(&data.collections))
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
