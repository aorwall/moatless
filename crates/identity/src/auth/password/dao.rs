//! Persistence for `user_credentials`.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::models::UserCredential;
use moatless_core::db::DbPool;

/// DAO trait for local credential persistence.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait UserCredentialDao: Send + Sync {
    /// Look up a credential through its user's normalized email.
    async fn find_by_email(&self, email: &str) -> Result<Option<UserCredential>, sqlx::Error>;
    async fn find_by_user_id(&self, user_id: &Uuid) -> Result<Option<UserCredential>, sqlx::Error>;
    /// Insert or replace a user's credential.
    async fn upsert(
        &self,
        user_id: &Uuid,
        username: &str,
        password_hash: &str,
        must_change: bool,
    ) -> Result<(), sqlx::Error>;
    /// Replace the hash and clear the failure counter and change flag.
    async fn set_password_hash(
        &self,
        user_id: &Uuid,
        password_hash: &str,
    ) -> Result<bool, sqlx::Error>;
    async fn record_login_success(
        &self,
        user_id: &Uuid,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;
    async fn record_login_failure(&self, user_id: &Uuid) -> Result<(), sqlx::Error>;
    async fn delete(&self, user_id: &Uuid) -> Result<bool, sqlx::Error>;
}

/// PostgreSQL implementation of [`UserCredentialDao`].
pub struct PgUserCredentialDao(pub DbPool);

#[async_trait]
impl UserCredentialDao for PgUserCredentialDao {
    async fn find_by_email(&self, email: &str) -> Result<Option<UserCredential>, sqlx::Error> {
        sqlx::query_as::<_, UserCredential>(
            "SELECT c.user_id, c.username, c.password_hash, c.must_change, c.failed_attempts, \
                    c.last_login_at, c.password_changed_at, c.created_at, c.updated_at \
             FROM user_credentials c \
             JOIN users u ON u.id = c.user_id \
             WHERE u.email_normalized = lower(btrim($1)) AND NOT u.is_bot",
        )
        .bind(email)
        .fetch_optional(&self.0)
        .await
    }

    async fn find_by_user_id(&self, user_id: &Uuid) -> Result<Option<UserCredential>, sqlx::Error> {
        sqlx::query_as::<_, UserCredential>(
            "SELECT user_id, username, password_hash, must_change, failed_attempts, \
                    last_login_at, password_changed_at, created_at, updated_at \
             FROM user_credentials WHERE user_id = $1",
        )
        .bind(user_id)
        .fetch_optional(&self.0)
        .await
    }

    async fn upsert(
        &self,
        user_id: &Uuid,
        username: &str,
        password_hash: &str,
        must_change: bool,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"INSERT INTO user_credentials
                   (user_id, username, password_hash, must_change, password_changed_at, updated_at)
               VALUES ($1, $2, $3, $4, now(), now())
               ON CONFLICT (user_id) DO UPDATE SET
                   username            = EXCLUDED.username,
                   password_hash       = EXCLUDED.password_hash,
                   must_change         = EXCLUDED.must_change,
                   failed_attempts     = 0,
                   password_changed_at = now(),
                   updated_at          = now()"#,
        )
        .bind(user_id)
        .bind(username)
        .bind(password_hash)
        .bind(must_change)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn set_password_hash(
        &self,
        user_id: &Uuid,
        password_hash: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"UPDATE user_credentials
               SET password_hash       = $2,
                   must_change         = false,
                   failed_attempts     = 0,
                   password_changed_at = now(),
                   updated_at          = now()
               WHERE user_id = $1"#,
        )
        .bind(user_id)
        .bind(password_hash)
        .execute(&self.0)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn record_login_success(
        &self,
        user_id: &Uuid,
        at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE user_credentials
               SET last_login_at = $2, failed_attempts = 0, updated_at = now()
               WHERE user_id = $1"#,
        )
        .bind(user_id)
        .bind(at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn record_login_failure(&self, user_id: &Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"UPDATE user_credentials
               SET failed_attempts = failed_attempts + 1, updated_at = now()
               WHERE user_id = $1"#,
        )
        .bind(user_id)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn delete(&self, user_id: &Uuid) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM user_credentials WHERE user_id = $1")
            .bind(user_id)
            .execute(&self.0)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
