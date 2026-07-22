//! Seed the bootstrap admin for local password auth.

use chrono::Utc;
use thiserror::Error;
use tracing::{info, warn};

use super::dao::UserCredentialRepository;
use super::hash::{generate_password, hash_password, PasswordHashError};
use crate::auth::models::UserRole;
use crate::auth::users::{User, UserRepository};
use crate::config::AuthConfig;

/// Length of a generated bootstrap password.
const GENERATED_PASSWORD_LEN: usize = 24;

#[derive(Debug, Error)]
pub enum BootstrapError {
    #[error("failed to read existing admin credential: {0}")]
    Read(sqlx::Error),
    #[error("failed to persist admin user: {0}")]
    Persist(sqlx::Error),
    #[error(transparent)]
    Hash(#[from] PasswordHashError),
}

/// Ensure the local admin account exists and can log in.
///
/// Behaviour by `(ADMIN_PASSWORD, existing credential)`:
/// - `(Some, _)` — the password is declarative; the hash is re-upserted every boot.
/// - `(None, Some)` — an admin already exists; left untouched.
/// - `(None, None)` — a password is generated, stored hashed, and logged **once**
///   at WARN. The plaintext is never persisted.
pub async fn ensure_local_admin(
    users: &dyn UserRepository,
    credentials: &dyn UserCredentialRepository,
    config: &AuthConfig,
) -> Result<(), BootstrapError> {
    let username = config.admin_username.trim();
    // The `local:<username>` handle is the provider identity. Reuse the internal
    // id already mapped to it (an admin seeded before the uuid migration keeps
    // the id that migration assigned); only mint the deterministic v5 id for a
    // brand-new admin, so the seed stays idempotent across restarts.
    let provider_user_id = format!("local:{}", username.to_lowercase());
    let user_id = match users
        .find_user_by_provider_id("local", &provider_user_id)
        .await
        .map_err(BootstrapError::Read)?
    {
        Some(existing) => existing.id,
        None => uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, provider_user_id.as_bytes()),
    };

    let existing = credentials
        .find_by_user_id(&user_id)
        .await
        .map_err(BootstrapError::Read)?;

    let (password, must_change, generated) = match (&config.admin_password, &existing) {
        (Some(pw), _) => (pw.clone(), false, false),
        (None, Some(_)) => return Ok(()),
        (None, None) => (generate_password(GENERATED_PASSWORD_LEN), true, true),
    };

    let password_hash = hash_password(password.clone()).await?;

    let now = Utc::now();
    let user = User {
        id: user_id,
        login: username.to_string(),
        email: None,
        name: Some(username.to_string()),
        avatar_url: None,
        provider: "local".to_string(),
        provider_user_id: Some(provider_user_id.clone()),
        role: UserRole::Admin,
        created_at: now,
        last_login_at: now,
        settings: serde_json::json!({}),
        is_bot: false,
    };

    users
        .upsert_user(&user)
        .await
        .map_err(BootstrapError::Persist)?;

    // Keep the `local:<username>` identity resolvable (legacy refs, re-login).
    users
        .upsert_provider_id(&user_id, "local", &provider_user_id)
        .await
        .map_err(BootstrapError::Persist)?;

    credentials
        .upsert(&user_id, username, &password_hash, must_change)
        .await
        .map_err(BootstrapError::Persist)?;

    if generated {
        warn!(
            username = %username,
            password = %password,
            "No ADMIN_PASSWORD set — generated a bootstrap admin password. This is logged \
             once and cannot be recovered; sign in and change it, then set ADMIN_PASSWORD \
             or rely on the stored credential."
        );
    } else {
        info!(username = %username, "Bootstrap admin credential set from ADMIN_PASSWORD");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::password::dao::MockUserCredentialRepository;
    use crate::auth::password::hash::verify_password;
    use crate::auth::users::MockUserRepository;
    use crate::config::AuthProvider;
    use std::sync::{Arc, Mutex};

    fn auth_config(admin_password: Option<&str>) -> AuthConfig {
        let mut config = crate::config::Config::test_minimal().auth;
        config.provider = AuthProvider::Local;
        config.admin_username = "admin".into();
        config.admin_password = admin_password.map(str::to_string);
        config
    }

    /// Captures the arguments the bootstrap passes to `upsert`.
    type Captured = Arc<Mutex<Option<(String, String, String, bool)>>>;

    fn mocks(existing: bool) -> (MockUserRepository, MockUserCredentialRepository, Captured) {
        let captured: Captured = Arc::new(Mutex::new(None));

        let mut users = MockUserRepository::new();
        users.expect_upsert_user().returning(|_| Ok(()));
        users
            .expect_upsert_provider_id()
            .returning(|_, _, _| Ok(()));
        // No admin is mapped yet → the seed mints the deterministic v5 id.
        users
            .expect_find_user_by_provider_id()
            .returning(|_, _| Ok(None));

        let mut credentials = MockUserCredentialRepository::new();
        credentials
            .expect_find_by_user_id()
            .returning(move |user_id| {
                Ok(existing.then(|| super::super::models::UserCredential {
                    user_id: *user_id,
                    username: "admin".into(),
                    password_hash: "$argon2id$existing".into(),
                    must_change: false,
                    failed_attempts: 0,
                    last_login_at: None,
                    password_changed_at: Utc::now(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                }))
            });

        let sink = captured.clone();
        credentials
            .expect_upsert()
            .returning(move |user_id, username, hash, must_change| {
                *sink.lock().expect("captured mutex poisoned") = Some((
                    user_id.to_string(),
                    username.to_string(),
                    hash.to_string(),
                    must_change,
                ));
                Ok(())
            });

        (users, credentials, captured)
    }

    #[tokio::test]
    async fn configured_password_is_stored_hashed() {
        let (users, credentials, captured) = mocks(false);
        ensure_local_admin(&users, &credentials, &auth_config(Some("hunter2-hunter2")))
            .await
            .unwrap();

        let (user_id, username, hash, must_change) =
            captured.lock().unwrap().clone().expect("upsert not called");
        let expected_id =
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, b"local:admin").to_string();
        assert_eq!(user_id, expected_id);
        assert_eq!(username, "admin");
        assert!(!must_change);
        assert_ne!(hash, "hunter2-hunter2", "plaintext must never be persisted");
        assert!(verify_password("hunter2-hunter2".into(), hash).await.unwrap());
    }

    #[tokio::test]
    async fn existing_credential_is_left_alone_when_no_password_configured() {
        let (users, credentials, captured) = mocks(true);
        ensure_local_admin(&users, &credentials, &auth_config(None))
            .await
            .unwrap();
        assert!(
            captured.lock().unwrap().is_none(),
            "an existing admin credential must not be overwritten"
        );
    }

    #[tokio::test]
    async fn configured_password_overwrites_existing_credential() {
        let (users, credentials, captured) = mocks(true);
        ensure_local_admin(&users, &credentials, &auth_config(Some("new-password-1234")))
            .await
            .unwrap();
        let (_, _, hash, _) = captured.lock().unwrap().clone().expect("upsert not called");
        assert!(verify_password("new-password-1234".into(), hash).await.unwrap());
    }

    #[tokio::test]
    async fn existing_admin_keeps_its_mapped_id() {
        // An admin seeded before the uuid migration is mapped to a non-v5 id;
        // the seed must reuse it, not mint (and collide on) the v5 id.
        let mapped_id = uuid::Uuid::parse_str("fbeb5f73-28e2-4939-9cdb-b8f0107875a5").unwrap();
        let captured: Captured = Arc::new(Mutex::new(None));

        let mut users = MockUserRepository::new();
        users.expect_upsert_user().returning(|_| Ok(()));
        users
            .expect_upsert_provider_id()
            .returning(|_, _, _| Ok(()));
        users
            .expect_find_user_by_provider_id()
            .returning(move |_, _| {
                Ok(Some(User {
                    id: mapped_id,
                    login: "admin".into(),
                    email: None,
                    name: Some("admin".into()),
                    avatar_url: None,
                    provider: "local".into(),
                    provider_user_id: Some("local:admin".into()),
                    role: UserRole::Admin,
                    created_at: Utc::now(),
                    last_login_at: Utc::now(),
                    settings: serde_json::json!({}),
                    is_bot: false,
                }))
            });

        let mut credentials = MockUserCredentialRepository::new();
        credentials.expect_find_by_user_id().returning(|_| Ok(None));
        let sink = captured.clone();
        credentials
            .expect_upsert()
            .returning(move |user_id, _, _, _| {
                *sink.lock().expect("captured mutex poisoned") =
                    Some((user_id.to_string(), String::new(), String::new(), false));
                Ok(())
            });

        ensure_local_admin(&users, &credentials, &auth_config(Some("keep-my-id-1234")))
            .await
            .unwrap();

        let (user_id, ..) = captured.lock().unwrap().clone().expect("upsert not called");
        assert_eq!(user_id, mapped_id.to_string(), "must reuse the mapped id");
    }

    #[tokio::test]
    async fn generated_password_forces_a_change() {
        let (users, credentials, captured) = mocks(false);
        ensure_local_admin(&users, &credentials, &auth_config(None))
            .await
            .unwrap();
        let (_, _, hash, must_change) =
            captured.lock().unwrap().clone().expect("upsert not called");
        assert!(must_change, "a generated password must be flagged for rotation");
        assert!(hash.starts_with("$argon2id$"));
    }
}
