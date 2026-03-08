//! User table queries.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::models::UserRole;
use crate::db::DbPool;
use crate::shared_models::Settings;

#[derive(sqlx::FromRow, Debug)]
pub struct User {
    pub id: String,
    pub login: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub provider: String,
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
    pub last_login_at: DateTime<Utc>,
    pub settings: serde_json::Value,
}

impl User {
    /// Deserialize the JSONB `settings` column into the typed [`Settings`] struct.
    pub fn parsed_settings(&self) -> Settings {
        serde_json::from_value(self.settings.clone()).unwrap_or_default()
    }

    /// Extract the default channel repository ID from user settings.
    pub fn default_channel_repository_id(&self) -> Option<String> {
        self.parsed_settings().default_channel_repository_id
    }
}

/// Repository trait for user persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn get_user(&self, user_id: &str) -> Result<Option<User>, sqlx::Error>;
    async fn get_users_by_ids(&self, user_ids: &[String]) -> Result<Vec<User>, sqlx::Error>;
    async fn upsert_user(&self, user: &User) -> Result<(), sqlx::Error>;
    async fn list_users(&self) -> Result<Vec<User>, sqlx::Error>;
    async fn update_user_profile(
        &self,
        user_id: &str,
        name: Option<String>,
        email: Option<String>,
    ) -> Result<Option<User>, sqlx::Error>;

    /// Merge the given JSON object into the user's `settings` JSONB column.
    async fn update_user_settings(
        &self,
        user_id: &str,
        patch: &serde_json::Value,
    ) -> Result<Option<User>, sqlx::Error>;
}

/// PostgreSQL implementation of [`UserRepository`].
pub struct PgUserRepository(pub DbPool);

#[async_trait]
impl UserRepository for PgUserRepository {
    async fn get_user(&self, user_id: &str) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.0)
            .await
    }

    async fn get_users_by_ids(&self, user_ids: &[String]) -> Result<Vec<User>, sqlx::Error> {
        if user_ids.is_empty() {
            return Ok(Vec::new());
        }
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = ANY($1)")
            .bind(user_ids)
            .fetch_all(&self.0)
            .await
    }

    async fn upsert_user(&self, user: &User) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            INSERT INTO users (id, login, email, name, avatar_url, provider, role, created_at, last_login_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (id) DO UPDATE SET
                login = EXCLUDED.login,
                email = EXCLUDED.email,
                name = EXCLUDED.name,
                avatar_url = EXCLUDED.avatar_url,
                last_login_at = EXCLUDED.last_login_at
            "#,
        )
        .bind(&user.id)
        .bind(&user.login)
        .bind(&user.email)
        .bind(&user.name)
        .bind(&user.avatar_url)
        .bind(&user.provider)
        .bind(user.role)
        .bind(user.created_at)
        .bind(user.last_login_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn list_users(&self) -> Result<Vec<User>, sqlx::Error> {
        sqlx::query_as::<_, User>("SELECT * FROM users ORDER BY login ASC")
            .fetch_all(&self.0)
            .await
    }

    async fn update_user_profile(
        &self,
        user_id: &str,
        name: Option<String>,
        email: Option<String>,
    ) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as::<_, User>(
            "UPDATE users SET name = COALESCE($2, name), email = COALESCE($3, email) WHERE id = $1 RETURNING *",
        )
        .bind(user_id)
        .bind(name)
        .bind(email)
        .fetch_optional(&self.0)
        .await
    }

    async fn update_user_settings(
        &self,
        user_id: &str,
        patch: &serde_json::Value,
    ) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as::<_, User>(
            "UPDATE users SET settings = settings || $2 WHERE id = $1 RETURNING *",
        )
        .bind(user_id)
        .bind(patch)
        .fetch_optional(&self.0)
        .await
    }
}
