//! Session table queries.

use super::DbPool;
use chrono::NaiveDateTime;

#[derive(sqlx::FromRow, Debug)]
pub struct SessionRow {
    pub id: String,
    pub provider: String,
    pub user_id: String,
    pub user_login: String,
    pub user_email: Option<String>,
    pub user_name: Option<String>,
    pub avatar_url: Option<String>,
    pub access_token: Option<String>,
    pub id_token: Option<String>,
    pub token_type: Option<String>,
    pub scope: Option<String>,
    pub role: String,
    pub group_id: Option<String>,
    pub created_at: NaiveDateTime,
    pub expires_at: NaiveDateTime,
}

/// Get session by ID.
pub async fn get_session(
    pool: &DbPool,
    session_id: &str,
) -> Result<Option<SessionRow>, sqlx::Error> {
    sqlx::query_as::<_, SessionRow>("SELECT * FROM sessions WHERE id = $1")
        .bind(session_id)
        .fetch_optional(pool)
        .await
}

/// Insert a new session.
pub async fn insert_session(pool: &DbPool, session: &SessionRow) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO sessions (
            id, provider, user_id, user_login, user_email, user_name,
            avatar_url, access_token, id_token, token_type, scope,
            role, group_id, created_at, expires_at
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
        "#,
    )
    .bind(&session.id)
    .bind(&session.provider)
    .bind(&session.user_id)
    .bind(&session.user_login)
    .bind(&session.user_email)
    .bind(&session.user_name)
    .bind(&session.avatar_url)
    .bind(&session.access_token)
    .bind(&session.id_token)
    .bind(&session.token_type)
    .bind(&session.scope)
    .bind(&session.role)
    .bind(&session.group_id)
    .bind(session.created_at)
    .bind(session.expires_at)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete a session by ID.
pub async fn delete_session(pool: &DbPool, session_id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM sessions WHERE id = $1")
        .bind(session_id)
        .execute(pool)
        .await?;
    Ok(())
}
