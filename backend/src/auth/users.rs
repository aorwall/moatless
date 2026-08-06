//! User table queries.
//!
//! `mockall::automock` mirrors every trait signature below into a mock whose
//! matcher parameters trip lints on argument shapes we chose deliberately. The
//! generated item lands in this module, so this is the only scope that can
//! carry the suppression.
#![cfg_attr(
    test,
    expect(
        clippy::option_option,
        reason = "`update_user_profile_and_role` takes the PATCH body's tri-state \
                  (absent, null, value); mockall mirrors it into the mock"
    )
)]

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::models::UserRole;
use moatless_core::db::DbPool;

#[derive(sqlx::FromRow, Debug)]
pub struct User {
    pub id: Uuid,
    pub login: String,
    pub email: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub provider: String,
    /// The provider's own user id (GitHub numeric id, `local:<login>`,
    /// `bot:<appkey>`, or an Auth0 `sub`). Preserved so login can resolve
    /// `(provider, provider_user_id)` to this internal uuid.
    pub provider_user_id: Option<String>,
    pub role: UserRole,
    pub created_at: DateTime<Utc>,
    pub last_login_at: DateTime<Utc>,
    pub settings: serde_json::Value,
    /// `true` when this user row represents a GitHub App bot identity
    /// (`{slug}[bot]`). The App key + token-source is stored in the user's
    /// `provider_tokens` row (`auth_method = 'github_app_installation'`,
    /// `provider_user_id = <app_key>`).
    pub is_bot: bool,
}

/// A provider-specific identity linked to a moatless user.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct UserProviderAccount {
    pub user_id: Uuid,
    pub provider: String,
    pub provider_user_id: String,
    pub created_at: DateTime<Utc>,
}

/// DAO trait for user persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait UserDao: Send + Sync {
    async fn get_user(&self, user_id: &Uuid) -> Result<Option<User>, sqlx::Error>;
    async fn get_user_by_login(&self, login: &str) -> Result<Option<User>, sqlx::Error>;
    async fn get_users_by_ids(&self, user_ids: &[Uuid]) -> Result<Vec<User>, sqlx::Error>;
    async fn upsert_user(&self, user: &User) -> Result<(), sqlx::Error>;
    async fn list_users(&self) -> Result<Vec<User>, sqlx::Error>;
    async fn update_user_profile(
        &self,
        user_id: &Uuid,
        name: Option<String>,
        email: Option<String>,
    ) -> Result<Option<User>, sqlx::Error>;
    /// `Option<Option<_>>` is the tri-state the PATCH body carries, not an
    /// accident: absent leaves the column alone, `null` clears it, a value sets
    /// it. The impl below binds `is_some()` and `flatten()` separately for
    /// exactly that reason. Collapsing it needs a patch type on the request
    /// body, which is a different change from a lint sweep.
    async fn update_user_profile_and_role(
        &self,
        user_id: &Uuid,
        name: Option<Option<String>>,
        email: Option<Option<String>>,
        role: Option<UserRole>,
    ) -> Result<Option<User>, sqlx::Error>;

    // Provider ID mapping
    async fn upsert_provider_id(
        &self,
        user_id: &Uuid,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<(), sqlx::Error>;
    async fn get_provider_id(
        &self,
        user_id: &Uuid,
        provider: &str,
    ) -> Result<Option<String>, sqlx::Error>;
    async fn find_user_by_provider_id(
        &self,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<Option<User>, sqlx::Error>;
    /// Resolve a legacy provider id (the old `users.id`, preserved in
    /// `user_provider_ids.provider_user_id`) to a user, regardless of provider.
    /// Used to keep pre-migration references (e.g. committed `runAsUserId`
    /// values) resolving to the new internal uuid.
    async fn find_user_by_legacy_id(
        &self,
        provider_user_id: &str,
    ) -> Result<Option<User>, sqlx::Error>;
    async fn delete_provider_id(&self, user_id: &Uuid, provider: &str)
        -> Result<bool, sqlx::Error>;
    async fn list_provider_ids(
        &self,
        user_id: &Uuid,
    ) -> Result<Vec<UserProviderAccount>, sqlx::Error>;
}

/// PostgreSQL implementation of [`UserDao`].
pub struct PgUserDao(pub DbPool);

#[async_trait]
impl UserDao for PgUserDao {
    async fn get_user(&self, user_id: &Uuid) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
            .bind(user_id)
            .fetch_optional(&self.0)
            .await
    }

    async fn get_users_by_ids(&self, user_ids: &[Uuid]) -> Result<Vec<User>, sqlx::Error> {
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
            INSERT INTO users (id, login, email, name, avatar_url, provider, provider_user_id, role, created_at, last_login_at, is_bot)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ON CONFLICT (id) DO UPDATE SET
                login = EXCLUDED.login,
                email = COALESCE(users.email, EXCLUDED.email),
                name = EXCLUDED.name,
                avatar_url = EXCLUDED.avatar_url,
                provider_user_id = COALESCE(EXCLUDED.provider_user_id, users.provider_user_id),
                last_login_at = EXCLUDED.last_login_at,
                is_bot = EXCLUDED.is_bot
            "#,
        )
        .bind(user.id)
        .bind(&user.login)
        .bind(&user.email)
        .bind(&user.name)
        .bind(&user.avatar_url)
        .bind(&user.provider)
        .bind(&user.provider_user_id)
        .bind(user.role)
        .bind(user.created_at)
        .bind(user.last_login_at)
        .bind(user.is_bot)
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
        user_id: &Uuid,
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

    async fn update_user_profile_and_role(
        &self,
        user_id: &Uuid,
        name: Option<Option<String>>,
        email: Option<Option<String>>,
        role: Option<UserRole>,
    ) -> Result<Option<User>, sqlx::Error> {
        let should_update_name = name.is_some();
        let should_update_email = email.is_some();
        sqlx::query_as::<_, User>(
            r#"UPDATE users
               SET name = CASE WHEN $2 THEN $3 ELSE name END,
                   email = CASE WHEN $4 THEN $5 ELSE email END,
                   role = COALESCE($6, role)
               WHERE id = $1
               RETURNING *"#,
        )
        .bind(user_id)
        .bind(should_update_name)
        .bind(name.flatten())
        .bind(should_update_email)
        .bind(email.flatten())
        .bind(role)
        .fetch_optional(&self.0)
        .await
    }

    async fn get_user_by_login(&self, login: &str) -> Result<Option<User>, sqlx::Error> {
        // `login` is not unique — the same handle can exist per provider.
        sqlx::query_as::<_, User>(
            "SELECT * FROM users WHERE login = $1 ORDER BY created_at ASC LIMIT 1",
        )
        .bind(login)
        .fetch_optional(&self.0)
        .await
    }

    async fn upsert_provider_id(
        &self,
        user_id: &Uuid,
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
        user_id: &Uuid,
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

    async fn find_user_by_legacy_id(
        &self,
        provider_user_id: &str,
    ) -> Result<Option<User>, sqlx::Error> {
        sqlx::query_as::<_, User>(
            r#"SELECT u.* FROM users u
               JOIN user_provider_ids p ON u.id = p.user_id
               WHERE p.provider_user_id = $1
               ORDER BY p.created_at ASC
               LIMIT 1"#,
        )
        .bind(provider_user_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn delete_provider_id(
        &self,
        user_id: &Uuid,
        provider: &str,
    ) -> Result<bool, sqlx::Error> {
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
        user_id: &Uuid,
    ) -> Result<Vec<UserProviderAccount>, sqlx::Error> {
        sqlx::query_as::<_, UserProviderAccount>(
            "SELECT * FROM user_provider_ids WHERE user_id = $1 ORDER BY provider ASC",
        )
        .bind(user_id)
        .fetch_all(&self.0)
        .await
    }
}
