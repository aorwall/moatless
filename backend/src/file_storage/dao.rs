//! File metadata DAO — queries for file upload metadata.

use async_trait::async_trait;

use super::models::FileMetadata;
use crate::db::DbPool;

/// Repository trait for file metadata persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait FileRepository: Send + Sync {
    async fn insert_metadata(&self, row: &FileMetadata) -> Result<(), sqlx::Error>;
    async fn find_by_id(&self, file_id: &str) -> Result<Option<FileMetadata>, sqlx::Error>;
    async fn delete_metadata(&self, file_id: &str) -> Result<bool, sqlx::Error>;
}

/// PostgreSQL implementation of [`FileRepository`].
pub struct PgFileRepository(pub DbPool);

#[async_trait]
impl FileRepository for PgFileRepository {
    async fn insert_metadata(&self, row: &FileMetadata) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO file_metadata (file_id, filename, mime_type, size, uploaded_at)
               VALUES ($1, $2, $3, $4, $5)"#,
        )
        .bind(&row.file_id)
        .bind(&row.filename)
        .bind(&row.mime_type)
        .bind(row.size)
        .bind(row.uploaded_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn find_by_id(&self, file_id: &str) -> Result<Option<FileMetadata>, sqlx::Error> {
        sqlx::query_as::<_, FileMetadata>(
            "SELECT * FROM file_metadata WHERE file_id = $1",
        )
        .bind(file_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn delete_metadata(&self, file_id: &str) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM file_metadata WHERE file_id = $1")
            .bind(file_id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
