//! Authoritative browser auth-session persistence.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use uuid::Uuid;

use moatless_types::user::UserRole;

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

pub use super::sessions_dao::PgSessionDao;
