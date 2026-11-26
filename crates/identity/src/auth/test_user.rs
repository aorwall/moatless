//! Auto-create a test user and API key on startup.
//!
//! When `TEST_API_KEY` is set, upserts a dedicated test user with a fixed
//! identity and inserts the API key (if not already present). No OAuth login
//! required — the test user exists only for programmatic access.

use chrono::Utc;
use tracing::{info, warn};

use super::api_key::hash_key;
use super::api_keys::ApiKey;
use super::api_keys_dao::ApiKeyDao;
use super::users::User;
use super::users_dao::UserDao;
use moatless_types::user::{UserRole, UserStatus};

const TEST_USER_ID: &str = "test-api-user";
const TEST_USER_LOGIN: &str = "test-api-user";
const TEST_USER_PROVIDER: &str = "api-key";
const TEST_KEY_NAME: &str = "test-api-key";

/// Ensures a test user and API key exist for the given raw `mvk_` key.
///
/// Idempotent: the user is upserted and the key is skipped if the hash already
/// exists.
pub(crate) async fn ensure_test_user(users: &dyn UserDao, api_keys: &dyn ApiKeyDao, raw_key: &str) {
    // Validate key format (already validated by Config, but belt-and-suspenders)
    if !raw_key.starts_with("mvk_") || raw_key.len() < 36 {
        warn!("TEST_API_KEY must start with 'mvk_' and be at least 36 characters — skipping");
        return;
    }

    // 1. Upsert the test user. The fixed `test-api-user` handle is the provider
    // identity; the internal id is a uuid derived from it (v5) for idempotency.
    let now = Utc::now();
    let user_id = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, TEST_USER_ID.as_bytes());
    let user = User {
        id: user_id,
        login: TEST_USER_LOGIN.to_string(),
        email: None,
        name: Some("Test API User".to_string()),
        avatar_url: None,
        provider: TEST_USER_PROVIDER.to_string(),
        provider_user_id: Some(TEST_USER_ID.to_string()),
        role: UserRole::Admin,
        created_at: now,
        last_login_at: now,
        settings: serde_json::json!({}),
        is_bot: false,
        status: UserStatus::Active,
    };

    if let Err(e) = users.upsert_user(&user).await {
        warn!(error = %e, "Failed to upsert test API user");
        return;
    }

    let key_hash = hash_key(raw_key);
    match api_keys.get_api_key_by_hash(&key_hash).await {
        Ok(Some(_)) => {
            let prefix = &raw_key[..raw_key.len().min(12)];
            info!(prefix = %prefix, user_id = TEST_USER_ID, "Test API key already exists — skipping insert");
            return;
        }
        Err(e) => {
            warn!(error = %e, "Failed to check existing test API key");
            return;
        }
        Ok(None) => { /* key not found, proceed to insert */ }
    }

    let key_prefix = format!("mvk_{}", &raw_key[4..raw_key.len().min(12)]);
    let id = format!("test-{}", &key_hash[..16]);

    let row = ApiKey {
        id,
        user_id: user_id.to_string(),
        name: TEST_KEY_NAME.to_string(),
        key_hash,
        key_prefix: key_prefix.clone(),
        scopes: None,
        last_used_at: None,
        expires_at: None,
        idle_ttl_secs: None,
        created_at: now,
    };

    if let Err(e) = api_keys.insert_api_key(&row).await {
        warn!(error = %e, "Failed to insert test API key");
        return;
    }

    info!(prefix = %key_prefix, user_id = TEST_USER_ID, "Test API user and key created");
}
