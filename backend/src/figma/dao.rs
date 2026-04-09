//! Figma import DAO — queries for figma import metadata.

use async_trait::async_trait;

use super::models::FigmaImport;
use crate::db::DbPool;

/// Repository trait for figma import persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait FigmaRepository: Send + Sync {
    async fn insert(&self, row: &FigmaImport) -> Result<(), sqlx::Error>;
    async fn find_by_id(&self, import_id: &str) -> Result<Option<FigmaImport>, sqlx::Error>;
    async fn delete(&self, import_id: &str) -> Result<bool, sqlx::Error>;
}

/// PostgreSQL implementation of [`FigmaRepository`].
pub struct PgFigmaRepository(pub DbPool);

#[async_trait]
impl FigmaRepository for PgFigmaRepository {
    async fn insert(&self, row: &FigmaImport) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO figma_imports (
                import_id, file_id, file_key, node_id, file_name, node_name,
                node_metadata, has_image, include_image_in_prompt, created_by, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)"#,
        )
        .bind(&row.import_id)
        .bind(&row.file_id)
        .bind(&row.file_key)
        .bind(&row.node_id)
        .bind(&row.file_name)
        .bind(&row.node_name)
        .bind(&row.node_metadata)
        .bind(row.has_image)
        .bind(row.include_image_in_prompt)
        .bind(&row.created_by)
        .bind(row.created_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn find_by_id(&self, import_id: &str) -> Result<Option<FigmaImport>, sqlx::Error> {
        sqlx::query_as::<_, FigmaImport>("SELECT * FROM figma_imports WHERE import_id = $1")
            .bind(import_id)
            .fetch_optional(&self.0)
            .await
    }

    async fn delete(&self, import_id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM figma_imports WHERE import_id = $1")
            .bind(import_id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
