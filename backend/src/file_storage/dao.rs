//! File metadata DAO — queries for file upload metadata.

use super::models::FileMetadata;
use crate::db::DbPool;

/// Insert file metadata.
pub async fn insert_metadata(pool: &DbPool, row: &FileMetadata) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"INSERT INTO file_metadata (file_id, filename, mime_type, size, uploaded_at)
           VALUES ($1, $2, $3, $4, $5)"#,
    )
    .bind(&row.file_id)
    .bind(&row.filename)
    .bind(&row.mime_type)
    .bind(row.size)
    .bind(row.uploaded_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Find file metadata by ID.
pub async fn find_by_id(pool: &DbPool, file_id: &str) -> Result<Option<FileMetadata>, sqlx::Error> {
    sqlx::query_as::<_, FileMetadata>(
        "SELECT file_id, filename, mime_type, size, uploaded_at FROM file_metadata WHERE file_id = $1",
    )
    .bind(file_id)
    .fetch_optional(pool)
    .await
}

/// Delete file metadata by ID.
pub async fn delete_metadata(pool: &DbPool, file_id: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM file_metadata WHERE file_id = $1")
        .bind(file_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
