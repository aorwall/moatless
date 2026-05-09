//! User table queries.

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use super::models::UserRole;
use crate::db::DbPool;

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

/// A provider-specific identity linked to a moatless user.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct UserProviderAccount {
    pub user_id: String,
    pub provider: String,
    pub provider_user_id: String,
    pub created_at: DateTime<Utc>,
}

/// Repository trait for user persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn get_user(&self, user_id: &str) -> Result<Option<User>, sqlx::Error>;
    async fn get_user_by_login(&self, login: &str) -> Result<Option<User>, sqlx::Error>;
    async fn get_users_by_ids(&self, user_ids: &[String]) -> Result<Vec<User>, sqlx::Error>;
    async fn upsert_user(&self, user: &User) -> Result<(), sqlx::Error>;
    async fn list_users(&self) -> Result<Vec<User>, sqlx::Error>;
    async fn update_user_profile(
        &self,
        user_id: &str,
        name: Option<String>,
        email: Option<String>,
    ) -> Result<Option<User>, sqlx::Error>;

    // Provider ID mapping
    async fn upsert_provider_id(
        &self,
        user_id: &str,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<(), sqlx::Error>;
    async fn get_provider_id(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<String>, sqlx::Error>;
    async fn find_user_by_provider_id(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<User>, sqlx::Error>;
    async fn delete_provider_id(&self, user_id: &str, provider: &str) -> Result<bool, sqlx::Error>;
    async fn list_provider_ids(
        &self,
        user_id: &str,
    ) -> Result<Vec<UserProviderAccount>, sqlx::Error>;
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
                email = COALESCE(users.email, EXCLUDED.email),
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

    async fn get_user_by_login(&self, login: &str) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE login = $1")
            .bind(login)
            .fetch_optional(&self.0)
            .await
    }

    async fn upsert_provider_id(
        &self,
        user_id: &str,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO user_provider_ids (user_id, provider, provider_user_id)
               VALUES ($1, $2, $3)
               ON CONFLICT (user_id, provider) DO UPDATE
               SET provider_user_id = EXCLUDED.provider_user_id"#,
        )
        .bind(user_id)
        .bind(provider)
        .bind(provider_user_id)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn get_provider_id(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT provider_user_id FROM user_provider_ids WHERE user_id = $1 AND provider = $2",
        )
        .bind(user_id)
        .bind(provider)
        .fetch_optional(&self.0)
        .await?;
        Ok(row.map(|(id,)| id))
    }

    async fn find_user_by_provider_id(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as::<_, User>(
            r#"SELECT u.* FROM users u
               JOIN user_provider_ids p ON u.id = p.user_id
               WHERE p.provider = $1 AND p.provider_user_id = $2
               LIMIT 1"#,
        )
        .bind(provider)
        .bind(provider_user_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn delete_provider_id(&self, user_id: &str, provider: &str) -> Result<bool, sqlx::Error> {
        let result =
            sqlx::query("DELETE FROM user_provider_ids WHERE user_id = $1 AND provider = $2")
                .bind(user_id)
                .bind(provider)
                .execute(&self.0)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn list_provider_ids(
        &self,
        user_id: &str,
    ) -> Result<Vec<UserProviderAccount>, sqlx::Error> {
        sqlx::query_as::<_, UserProviderAccount>(
            "SELECT * FROM user_provider_ids WHERE user_id = $1 ORDER BY provider ASC",
        )
        .bind(user_id)
        .fetch_all(&self.0)
        .await
    }
}
