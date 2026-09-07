//! User table queries.
//!
//! `mockall::automock` mirrors every trait signature below into a mock whose
//! matcher parameters trip lints on argument shapes we chose deliberately. The
//! generated item lands in this module, so this is the only scope that can
//! carry the suppression.
#![cfg_attr(
    any(test, feature = "test-support"),
    expect(
        clippy::option_option,
        reason = "`update_user_profile_and_role` takes the PATCH body's tri-state \
                  (absent, null, value); mockall mirrors it into the mock"
    )
)]

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use uuid::Uuid;

use moatless_types::user::UserRole;

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
    #[sqlx(try_from = "String")]
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
#[cfg_attr(any(test, feature = "test-support"), mockall::automock)]
#[async_trait]
pub trait UserDao: Send + Sync {
    async fn get_user(&self, user_id: &Uuid) -> Result<Option<User>, sqlx::Error>;
    async fn get_user_by_login(&self, login: &str) -> Result<Option<User>, sqlx::Error>;
    async fn find_user_by_email(&self, email: &str) -> Result<Option<User>, sqlx::Error>;
    async fn get_users_by_ids(&self, user_ids: &[Uuid]) -> Result<Vec<User>, sqlx::Error>;
    async fn upsert_user(&self, user: &User) -> Result<(), sqlx::Error>;
    async fn list_users(&self) -> Result<Vec<User>, sqlx::Error>;
    async fn list_bot_users_by_app_key(&self) -> Result<HashMap<String, String>, sqlx::Error>;
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
    /// Record `provider_user_id` only when the user holds no id at `provider`
    /// yet. For a caller caching what it just resolved: a link already there was
    /// established deliberately and outranks a fresh sighting.
    async fn insert_provider_id_if_absent(
        &self,
        user_id: &Uuid,
        provider: &str,
        provider_user_id: &str,
    ) -> Result<bool, sqlx::Error>;
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

pub use super::users_dao::PgUserDao;
