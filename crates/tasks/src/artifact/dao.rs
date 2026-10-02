//! Artifact DAO — queries for `task_artifacts`.

use async_trait::async_trait;

use super::models::{Artifact, Reservation};
use moatless_core::db::DbPool;

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
    /// Add `version` to `published_versions` and return the row, or `None` when no row
    /// carries `artifact_id`. A version older than the current latest one
    /// leaves `latest_version` and `title` as they are.
    async fn commit_version(
        &self,
        artifact_id: &str,
        version: i32,
        title: &str,
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
    ) -> Result<Option<Artifact>, sqlx::Error> {
        sqlx::query_as::<_, Artifact>(
            r#"UPDATE task_artifacts SET
                   title = CASE WHEN $2 > latest_version THEN $3 ELSE title END,
                   latest_version = GREATEST(latest_version, $2),
                   published_versions = array_append(published_versions, $2),
                   updated_at = now()
               WHERE artifact_id = $1
               RETURNING *"#,
        )
        .bind(artifact_id)
        .bind(version)
        .bind(title)
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

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_db_tests.rs"]
mod dao_db_tests;
