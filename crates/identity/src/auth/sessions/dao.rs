//! Authoritative browser auth-session persistence.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moatless_core::db::DbPool;
use uuid::Uuid;

use moatless_types::user::UserRole;

/// A newly issued auth session. The browser receives the raw token; this row
/// contains only its SHA-256 digest.
#[derive(Debug)]
pub(in crate::auth) struct NewAuthSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token_hash: Vec<u8>,
    pub csrf_token_hash: Vec<u8>,
    pub authenticated_via: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
    pub idle_expires_at: DateTime<Utc>,
    pub absolute_expires_at: DateTime<Utc>,
}

/// The current account and session facts returned for a valid opaque token.
#[derive(sqlx::FromRow, Debug, Clone)]
pub(in crate::auth) struct AuthenticatedSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub login: String,
    #[sqlx(try_from = "String")]
    pub role: UserRole,
    pub authenticated_via: String,
    pub csrf_token_hash: Vec<u8>,
    pub last_seen_at: DateTime<Utc>,
    pub idle_expires_at: DateTime<Utc>,
    pub absolute_expires_at: DateTime<Utc>,
}

/// Persistence seam exercised by auth route and middleware tests.
#[cfg_attr(test, mockall::automock)]
#[async_trait]
pub(in crate::auth) trait SessionDao: Send + Sync {
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
pub(in crate::auth) struct PgSessionDao(pub(in crate::auth) DbPool);

#[async_trait]
impl SessionDao for PgSessionDao {
    async fn insert_session(&self, session: &NewAuthSession) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            WITH inserted AS (
                INSERT INTO auth_sessions (
                    id, user_id, token_hash, csrf_token_hash, authenticated_via,
                    created_at, last_seen_at, idle_expires_at, absolute_expires_at
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
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
        .bind(session.created_at)
        .bind(session.last_seen_at)
        .bind(session.idle_expires_at)
        .bind(session.absolute_expires_at)
        .execute(&self.0)
        .await?;
        Ok(())
    }

    // Bot sessions are issued by token/API-key exchange; filtering bots here
    // rejects the sandbox CLI after its user-token JWT expires.
    async fn find_active_by_token_hash(
        &self,
        token_hash: &[u8],
        now: DateTime<Utc>,
    ) -> Result<Option<AuthenticatedSession>, sqlx::Error> {
        sqlx::query_as::<_, AuthenticatedSession>(
            r#"
            SELECT s.id, s.user_id, u.login, u.role, s.authenticated_via,
                   s.csrf_token_hash, s.last_seen_at, s.idle_expires_at,
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

#[cfg(all(test, feature = "db-tests"))]
#[path = "dao_tests.rs"]
mod db_tests;

#[cfg(test)]
mod tests {
    use super::{NewAuthSession, PgSessionDao, SessionDao};
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    #[tokio::test]
    async fn a_closed_database_preserves_errors_on_every_dao_operation() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/postgres")
            .unwrap();
        pool.close().await;
        let sessions = PgSessionDao(pool);
        let user_id = Uuid::new_v4();
        let now = Utc::now();
        let new_session = NewAuthSession {
            id: Uuid::new_v4(),
            user_id,
            token_hash: b"closed".to_vec(),
            csrf_token_hash: b"csrf-hash".to_vec(),
            authenticated_via: "token".to_string(),
            created_at: now,
            last_seen_at: now,
            idle_expires_at: now + Duration::hours(1),
            absolute_expires_at: now + Duration::days(1),
        };
        assert!(matches!(
            sessions.insert_session(&new_session).await,
            Err(sqlx::Error::PoolClosed)
        ));
        assert!(matches!(
            sessions.find_active_by_token_hash(b"closed", now).await,
            Err(sqlx::Error::PoolClosed)
        ));
        assert!(matches!(
            sessions.touch(Uuid::new_v4(), now, now).await,
            Err(sqlx::Error::PoolClosed)
        ));
        assert!(matches!(
            sessions.revoke_by_token_hash(b"closed", now).await,
            Err(sqlx::Error::PoolClosed)
        ));
        assert!(matches!(
            sessions
                .delete_sessions_for_user(&user_id.to_string())
                .await,
            Err(sqlx::Error::PoolClosed)
        ));
    }
}
