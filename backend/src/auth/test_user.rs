//! Auto-create a test user and API key on startup.
//!
//! When `TEST_API_KEY` is set, upserts a dedicated test user with a fixed
//! identity and inserts the API key (if not already present). No OAuth
//! login required — the test user exists only for programmatic access.

use chrono::Utc;
use tracing::{info, warn};

use super::api_key::hash_key;
use super::api_keys::{ApiKey, ApiKeyRepository};
use super::models::UserRole;
use super::users::{User, UserRepository};

const TEST_USER_ID: &str = "test-api-user";
const TEST_USER_LOGIN: &str = "test-api-user";
const TEST_USER_PROVIDER: &str = "api-key";
const TEST_KEY_NAME: &str = "test-api-key";

/// Ensure a test user and API key exist for the given raw `mvk_` key.
///
/// Idempotent: the user is upserted and the key is skipped if the hash
/// already exists.
pub async fn ensure_test_user(
    users: &dyn UserRepository,
    api_keys: &dyn ApiKeyRepository,
    raw_key: &str,
) {
    // Validate key format (already validated by Config, but belt-and-suspenders)
    if !raw_key.starts_with("mvk_") || raw_key.len() < 36 {
        warn!("TEST_API_KEY must start with 'mvk_' and be at least 36 characters — skipping");
        return;
    }

    // 1. Upsert the test user
    let now = Utc::now();
    let user = User {
        id: TEST_USER_ID.to_string(),
        login: TEST_USER_LOGIN.to_string(),
        email: None,
        name: Some("Test API User".to_string()),
        avatar_url: None,
        provider: TEST_USER_PROVIDER.to_string(),
        role: UserRole::Admin,
        created_at: now,
        last_login_at: now,
        settings: serde_json::json!({}),
        is_bot: false,
    };

    if let Err(e) = users.upsert_user(&user).await {
        warn!(error = %e, "Failed to upsert test API user");
        return;
    }

    // 2. Check if API key hash already exists → skip if so
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

    // 3. Insert the API key
    let key_prefix = format!("mvk_{}", &raw_key[4..raw_key.len().min(12)]);
    let id = format!("test-{}", &key_hash[..16]);

    let row = ApiKey {
        id,
        user_id: TEST_USER_ID.to_string(),
        name: TEST_KEY_NAME.to_string(),
        key_hash,
        key_prefix: key_prefix.clone(),
        scopes: None,
        group_id: None,
        last_used_at: None,
        expires_at: None,
        created_at: now,
    };

    if let Err(e) = api_keys.insert_api_key(&row).await {
        warn!(error = %e, "Failed to insert test API key");
        return;
    }

    info!(prefix = %key_prefix, user_id = TEST_USER_ID, "Test API user and key created");
}
