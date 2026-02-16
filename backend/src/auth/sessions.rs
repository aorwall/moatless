//! Session table queries.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::models::UserRole;
use crate::db::DbPool;

#[derive(sqlx::FromRow, Debug)]
pub struct Session {
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
    pub role: UserRole,
    pub group_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// Repository trait for session persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait SessionRepository: Send + Sync {
    async fn get_session(&self, session_id: &str) -> Result<Option<Session>, sqlx::Error>;
    async fn insert_session(&self, session: &Session) -> Result<(), sqlx::Error>;
    async fn delete_session(&self, session_id: &str) -> Result<(), sqlx::Error>;
}

/// PostgreSQL implementation of [`SessionRepository`].
pub struct PgSessionRepository(pub DbPool);

#[async_trait]
impl SessionRepository for PgSessionRepository {
    async fn get_session(&self, session_id: &str) -> Result<Option<Session>, sqlx::Error> {
        sqlx::query_as::<_, Session>("SELECT * FROM sessions WHERE id = $1")
            .bind(session_id)
            .fetch_optional(&self.0)
            .await
    }

    async fn insert_session(&self, session: &Session) -> Result<(), sqlx::Error> {
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
        .bind(session.role)
        .bind(&session.group_id)
        .bind(session.created_at)
        .bind(session.expires_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn delete_session(&self, session_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM sessions WHERE id = $1")
            .bind(session_id)
            .execute(&self.0)
            .await?;
        Ok(())
    }
}
