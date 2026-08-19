//! Authoritative browser auth-session persistence.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use moatless_types::user::UserRole;
use moatless_core::db::DbPool;

/// A newly issued auth session. The browser receives the raw token; this row
/// contains only its SHA-256 digest.
#[derive(Debug)]
pub struct NewAuthSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token_hash: Vec<u8>,
    pub csrf_token_hash: Vec<u8>,
    pub authenticated_via: String,
    pub group_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub idle_expires_at: DateTime<Utc>,
    pub absolute_expires_at: DateTime<Utc>,
}

/// The current account and session facts returned for a valid opaque token.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct AuthenticatedSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub login: String,
    #[sqlx(try_from = "String")]
    pub role: UserRole,
    pub authenticated_via: String,
    pub csrf_token_hash: Vec<u8>,
    pub group_id: Option<String>,
    pub last_seen_at: DateTime<Utc>,
    pub idle_expires_at: DateTime<Utc>,
    pub absolute_expires_at: DateTime<Utc>,
}

/// Persistence seam exercised by auth route and middleware tests.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub trait SessionDao: Send + Sync {
    async fn insert_session(&self, session: &NewAuthSession) -> Result<(), sqlx::Error>;
    async fn find_active_by_token_hash(
        &self,
        token_hash: &[u8],
        now: DateTime<Utc>,
    ) -> Result<Option<AuthenticatedSession>, sqlx::Error>;
    async fn touch(
        &self,
        session_id: Uuid,
        now: DateTime<Utc>,
        idle_expires_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error>;
    async fn revoke_by_token_hash(
        &self,
        token_hash: &[u8],
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error>;
    async fn delete_sessions_for_user(&self, user_id: &str) -> Result<u64, sqlx::Error>;
}

/// PostgreSQL implementation of [`SessionDao`].
pub struct PgSessionDao(pub DbPool);

#[async_trait]
impl SessionDao for PgSessionDao {
    async fn insert_session(&self, session: &NewAuthSession) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            WITH inserted AS (
                INSERT INTO auth_sessions (
                    id, user_id, token_hash, csrf_token_hash, authenticated_via,
                    group_id, created_at, last_seen_at, idle_expires_at,
                    absolute_expires_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                RETURNING id, user_id, authenticated_via
            )
            INSERT INTO auth_audit_events
                (id, user_id, auth_session_id, event, authentication_method, success)
            SELECT gen_random_uuid(), user_id, id, 'session_created', authenticated_via, true
            FROM inserted
            "#,
        )
        .bind(session.id)
        .bind(session.user_id)
        .bind(&session.token_hash)
        .bind(&session.csrf_token_hash)
        .bind(&session.authenticated_via)
        .bind(&session.group_id)
        .bind(session.created_at)
        .bind(session.last_seen_at)
        .bind(session.idle_expires_at)
        .bind(session.absolute_expires_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    // Validates an already-issued opaque session token. Deliberately does NOT
    // filter `NOT u.is_bot`: the interactive login paths (OAuth, password) block
    // bots at login, so the only sessions a bot ever holds are the ones the
    // token/api-key exchange endpoints issued on purpose for its sandbox CLI.
    // Filtering bots here rejected exactly those sessions, wedging a bot-owned
    // task's `moat` in a permanent 401 once its `X-User-Token` JWT expired.
    async fn find_active_by_token_hash(
        &self,
        token_hash: &[u8],
        now: DateTime<Utc>,
    ) -> Result<Option<AuthenticatedSession>, sqlx::Error> {
        sqlx::query_as::<_, AuthenticatedSession>(
            r#"
            SELECT s.id, s.user_id, u.login, u.role, s.authenticated_via,
                   s.csrf_token_hash, s.group_id, s.last_seen_at, s.idle_expires_at,
                   s.absolute_expires_at
            FROM auth_sessions s
            JOIN users u ON u.id = s.user_id
            WHERE s.token_hash = $1
              AND s.revoked_at IS NULL
              AND s.idle_expires_at > $2
              AND s.absolute_expires_at > $2
              AND u.status = 'active'
            "#,
        )
        .bind(token_hash)
        .bind(now)
        .fetch_optional(&self.0)
        .await
    }

    async fn touch(
        &self,
        session_id: Uuid,
        now: DateTime<Utc>,
        idle_expires_at: DateTime<Utc>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            UPDATE auth_sessions
            SET last_seen_at = $2,
                idle_expires_at = LEAST($3, absolute_expires_at)
            WHERE id = $1 AND revoked_at IS NULL
            "#,
        )
        .bind(session_id)
        .bind(now)
        .bind(idle_expires_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    async fn revoke_by_token_hash(
        &self,
        token_hash: &[u8],
        now: DateTime<Utc>,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"WITH revoked AS (
                   UPDATE auth_sessions
                   SET revoked_at = $2
                   WHERE token_hash = $1 AND revoked_at IS NULL
                   RETURNING id, user_id, authenticated_via
               )
               INSERT INTO auth_audit_events
                   (id, user_id, auth_session_id, event, authentication_method, success)
               SELECT gen_random_uuid(), user_id, id, 'session_revoked', authenticated_via, true
               FROM revoked"#,
        )
        .bind(token_hash)
        .bind(now)
        .execute(&self.0)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn delete_sessions_for_user(&self, user_id: &str) -> Result<u64, sqlx::Error> {
        let user_id = Uuid::parse_str(user_id).map_err(|error| {
            sqlx::Error::Protocol(format!("invalid auth-session user id: {error}"))
        })?;
        let result = sqlx::query(
            r#"WITH revoked AS (
                   UPDATE auth_sessions
                   SET revoked_at = now()
                   WHERE user_id = $1 AND revoked_at IS NULL
                   RETURNING id, user_id, authenticated_via
               )
               INSERT INTO auth_audit_events
                   (id, user_id, auth_session_id, event, authentication_method, success)
               SELECT gen_random_uuid(), user_id, id, 'session_revoked_all', authenticated_via, true
               FROM revoked"#,
        )
        .bind(user_id)
        .execute(&self.0)
        .await?;
        Ok(result.rows_affected())
    }
}
