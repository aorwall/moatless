//! Seed the bootstrap admin for local password auth.

use chrono::Utc;
use thiserror::Error;
use tracing::{info, warn};

use super::dao::UserCredentialDao;
use super::hash::{PasswordHashError, generate_password, hash_password};
use crate::auth::users::User;
use crate::auth::users_dao::UserDao;
use moatless_types::auth::AuthConfig;
use moatless_types::user::{UserRole, UserStatus};

/// Length of a generated bootstrap password.
const GENERATED_PASSWORD_LEN: usize = 24;

#[derive(Debug, Error)]
pub enum BootstrapError {
    #[error("ADMIN_EMAIL is required for the bootstrap administrator")]
    MissingEmail,
    #[error("failed to read existing admin credential: {0}")]
    Read(sqlx::Error),
    #[error("failed to persist admin user: {0}")]
    Persist(sqlx::Error),
    #[error(transparent)]
    Hash(#[from] PasswordHashError),
}

/// Ensure the local admin account exists and can log in.
///
/// An existing credential is never changed at startup; for a first-time seed,
/// `ADMIN_PASSWORD` is used when present, otherwise a random password is logged
/// once and marked for change.
pub(crate) async fn ensure_local_admin(
    users: &dyn UserDao,
    credentials: &dyn UserCredentialDao,
    config: &AuthConfig,
) -> Result<(), BootstrapError> {
    let username = config.admin_username.trim();
    let email = config
        .admin_email
        .as_deref()
        .map(str::trim)
        .filter(|email| email.contains('@'))
        .ok_or(BootstrapError::MissingEmail)?
        .to_lowercase();
    // The `local:<email>` handle is the provider identity. Reuse the internal
    // id already mapped to it (an admin seeded before the uuid migration keeps
    // the id that migration assigned); only mint the deterministic v5 id for a
    // brand-new admin, so the seed stays idempotent across restarts.
    let provider_user_id = format!("local:{email}");
    let existing_user = match users
        .find_user_by_email(&email)
        .await
        .map_err(BootstrapError::Read)?
    {
        Some(existing) => Some(existing),
        None => users
            .find_user_by_provider_id("local", &provider_user_id)
            .await
            .map_err(BootstrapError::Read)?,
    };
    let user_id = existing_user.as_ref().map_or_else(
        || uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, provider_user_id.as_bytes()),
        |existing| existing.id,
    );

    if existing_user.is_some() {
        users
            .update_user_profile_and_role(&user_id, None, None, Some(UserRole::Admin), None)
            .await
            .map_err(BootstrapError::Persist)?;
    }

    let existing = credentials
        .find_by_user_id(&user_id)
        .await
        .map_err(BootstrapError::Read)?;

    if existing.is_some() {
        return Ok(());
    }

    let (password, must_change, generated) = match &config.admin_password {
        Some(password) => (password.clone(), false, false),
        None => (generate_password(GENERATED_PASSWORD_LEN), true, true),
    };

    let password_hash = hash_password(password.clone()).await?;

    let now = Utc::now();
    let user = User {
        id: user_id,
        login: username.to_string(),
        email: Some(email.clone()),
        name: Some(username.to_string()),
        avatar_url: None,
        provider: "local".to_string(),
        provider_user_id: Some(provider_user_id.clone()),
        role: UserRole::Admin,
        created_at: now,
        last_login_at: now,
        settings: serde_json::json!({}),
        is_bot: false,
        status: UserStatus::Active,
    };

    if existing_user.is_none() {
        users
            .upsert_user(&user)
            .await
            .map_err(BootstrapError::Persist)?;
    }

    // Keep the `local:<email>` identity resolvable (legacy refs, re-login).
    users
        .upsert_provider_id(&user_id, "local", &provider_user_id)
        .await
        .map_err(BootstrapError::Persist)?;

    credentials
        .upsert(&user_id, &email, &password_hash, must_change)
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
    use crate::auth::password::dao::MockUserCredentialDao;
    use crate::auth::password::hash::verify_password;
    use crate::auth::users_dao::MockUserDao;
    use moatless_types::auth::AuthProvider;
    use std::sync::{Arc, Mutex};

    fn auth_config(admin_password: Option<&str>) -> AuthConfig {
        let mut config = moatless_core::config::Config::test_minimal().auth;
        config.provider = AuthProvider::Local;
        config.admin_username = "admin".into();
        config.admin_password = admin_password.map(str::to_string);
        config
    }

    /// Captures the arguments the bootstrap passes to `upsert`.
    type Captured = Arc<Mutex<Option<(String, String, String, bool)>>>;

    fn mocks(existing: bool) -> (MockUserDao, MockUserCredentialDao, Captured) {
        let captured: Captured = Arc::new(Mutex::new(None));

        let mut users = MockUserDao::new();
        users.expect_upsert_user().returning(|_| Ok(()));
        users
            .expect_upsert_provider_id()
            .returning(|_, _, _| Ok(()));
        // No admin is mapped yet → the seed mints the deterministic v5 id.
        users.expect_find_user_by_email().returning(|_| Ok(None));
        users
            .expect_find_user_by_provider_id()
            .returning(|_, _| Ok(None));

        let mut credentials = MockUserCredentialDao::new();
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
            uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, b"local:admin@example.com").to_string();
        assert_eq!(user_id, expected_id);
        assert_eq!(username, "admin@example.com");
        assert!(!must_change);
        assert_ne!(hash, "hunter2-hunter2", "plaintext must never be persisted");
        assert!(
            verify_password("hunter2-hunter2".into(), hash)
                .await
                .unwrap()
        );
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
    async fn configured_password_does_not_overwrite_existing_credential() {
        let (users, credentials, captured) = mocks(true);
        ensure_local_admin(
            &users,
            &credentials,
            &auth_config(Some("new-password-1234")),
        )
        .await
        .unwrap();
        assert!(captured.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn existing_admin_keeps_its_mapped_id() {
        // An admin seeded before the uuid migration is mapped to a non-v5 id;
        // the seed must reuse it, not mint (and collide on) the v5 id.
        let mapped_id = uuid::Uuid::parse_str("fbeb5f73-28e2-4939-9cdb-b8f0107875a5").unwrap();
        let captured: Captured = Arc::new(Mutex::new(None));

        let mut users = MockUserDao::new();
        users.expect_find_user_by_email().returning(|_| Ok(None));
        users.expect_upsert_user().never();
        users
            .expect_update_user_profile_and_role()
            .withf(move |user_id, name, email, role, status| {
                *user_id == mapped_id
                    && name.is_none()
                    && email.is_none()
                    && *role == Some(UserRole::Admin)
                    && status.is_none()
            })
            .returning(|_, _, _, _, _| Ok(None));
        users
            .expect_upsert_provider_id()
            .returning(|_, _, _| Ok(()));
        users
            .expect_find_user_by_provider_id()
            .returning(move |_, _| {
                Ok(Some(User {
                    id: mapped_id,
                    login: "admin".into(),
                    email: Some("admin@example.com".into()),
                    name: Some("admin".into()),
                    avatar_url: None,
                    provider: "local".into(),
                    provider_user_id: Some("local:admin".into()),
                    role: UserRole::Admin,
                    created_at: Utc::now(),
                    last_login_at: Utc::now(),
                    settings: serde_json::json!({}),
                    is_bot: false,
                    status: UserStatus::Active,
                }))
            });

        let mut credentials = MockUserCredentialDao::new();
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
        assert!(
            must_change,
            "a generated password must be flagged for rotation"
        );
        assert!(hash.starts_with("$argon2id$"));
    }
}
