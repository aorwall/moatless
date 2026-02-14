//! Figma import DAO — queries for figma import metadata.

use super::models::FigmaImport;
use crate::db::DbPool;

/// Insert a figma import record.
pub async fn insert(pool: &DbPool, row: &FigmaImport) -> Result<(), sqlx::Error> {
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
    .bind(&row.has_image)
    .bind(&row.include_image_in_prompt)
    .bind(&row.created_by)
    .bind(&row.created_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Find a figma import by ID.
pub async fn find_by_id(
    pool: &DbPool,
    import_id: &str,
) -> Result<Option<FigmaImport>, sqlx::Error> {
    sqlx::query_as::<_, FigmaImport>(
        r#"SELECT import_id, file_id, file_key, node_id, file_name, node_name,
                  node_metadata, has_image, include_image_in_prompt, created_by, created_at
           FROM figma_imports WHERE import_id = $1"#,
    )
    .bind(import_id)
    .fetch_optional(pool)
    .await
}

/// Delete a figma import by ID.
pub async fn delete(pool: &DbPool, import_id: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM figma_imports WHERE import_id = $1")
        .bind(import_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
