//! API key service — generate, validate, revoke.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use rand::RngExt;
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

use super::api_keys::ApiKey;
use super::api_keys_dao::ApiKeyDao;
use super::users_dao::UserDao;
use moatless_types::user::UserRole;

const KEY_PREFIX: &str = "mvk_";
const KEY_BYTES: usize = 32;

/// Payload extracted from a validated API key.
pub struct ApiKeyPayload {
    pub user_id: String,
    pub user_name: Option<String>,
    pub role: UserRole,
    pub group_id: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// Result of creating a new API key.
#[derive(serde::Serialize)]
pub struct ApiKeyCreateResult {
    pub id: String,
    pub key: String,
    pub name: String,
    pub key_prefix: String,
    pub scopes: Option<Vec<String>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Generate a new API key for a user.
pub(crate) async fn generate_key(
    api_keys: &dyn ApiKeyDao,
    user_id: &str,
    name: &str,
    scopes: Option<Vec<String>>,
    expires_at: Option<DateTime<Utc>>,
    group_id: Option<&str>,
) -> Result<ApiKeyCreateResult, crate::auth::error::AuthError> {
    let key_bytes: [u8; KEY_BYTES] = rand::rng().random();
    let key_hex = hex::encode(key_bytes);
    let full_key = format!("{KEY_PREFIX}{key_hex}");

    let key_hash = hash_key(&full_key);
    let key_prefix = format!("{}{}", KEY_PREFIX, &key_hex[..8]);

    // Random 16-byte hex ID
    let id_bytes: [u8; 16] = rand::rng().random();
    let id = hex::encode(id_bytes);

    let now = chrono::Utc::now();

    let row = ApiKey {
        id: id.clone(),
        user_id: user_id.to_string(),
        name: name.to_string(),
        key_hash,
        key_prefix: key_prefix.clone(),
        scopes: scopes
            .as_ref()
            .map(|s| serde_json::to_string(s).unwrap_or_default()),
        group_id: group_id.map(String::from),
        last_used_at: None,
        expires_at,
        created_at: now,
    };
    api_keys.insert_api_key(&row).await?;

    info!(user_id = %user_id, key_id = %id, key_prefix = %key_prefix, name = %name, "Created API key");

    Ok(ApiKeyCreateResult {
        id,
        key: full_key,
        name: name.to_string(),
        key_prefix,
        scopes,
        expires_at,
        created_at: now,
    })
}

/// Validate an API key. Returns payload if valid, None otherwise.
pub(crate) async fn validate_key(
    api_keys: Arc<dyn ApiKeyDao>,
    users: &dyn UserDao,
    key: &str,
) -> Result<Option<ApiKeyPayload>, crate::auth::error::AuthError> {
    if !key.starts_with(KEY_PREFIX) {
        return Ok(None);
    }

    let key_hash = hash_key(key);
    let row = api_keys.get_api_key_by_hash(&key_hash).await?;

    let Some(row) = row else {
        debug!(key_prefix = &key[..12], "API key not found");
        return Ok(None);
    };

    // Check expiration
    if let Some(exp) = row.expires_at {
        if chrono::Utc::now() > exp {
            debug!(key_id = %row.id, "API key expired");
            return Ok(None);
        }
    }

    // Update last_used_at (fire and forget)
    let api_keys_clone = api_keys.clone();
    let key_id_clone = row.id.clone();
    tokio::spawn(async move {
        if let Err(e) = api_keys_clone.update_last_used(&key_id_clone).await {
            warn!(error = %e, key_id = %key_id_clone, "Failed to update API key last used");
        }
    });

    // Get user info. The stored id is the uuid's text form.
    let user = match row.user_id.parse::<uuid::Uuid>() {
        Ok(uid) => users.get_user(&uid).await?,
        Err(_) => None,
    };
    let (user_name, role) = match user {
        Some(u) => (Some(u.login), u.role),
        None => (None, UserRole::User),
    };

    Ok(Some(ApiKeyPayload {
        user_id: row.user_id,
        user_name,
        role,
        group_id: row.group_id,
        expires_at: row.expires_at,
    }))
}

/// Validate an API key with cache-aside. Used by `verify_handler` only.
///
/// 1. Compute SHA-256 hash (CPU-only, no DB)
/// 2. Check cache by hash → return if hit (with expiry check)
/// 3. Miss → call `validate_key()` → build `VerifyPayload` → insert cache → return
pub(crate) async fn validate_key_cached(
    api_keys: Arc<dyn ApiKeyDao>,
    users: &dyn UserDao,
    key: &str,
    cache: &crate::auth::cache::AuthCache,
) -> Result<Option<crate::auth::cache::VerifyPayload>, crate::auth::error::AuthError> {
    if !key.starts_with(KEY_PREFIX) {
        return Ok(None);
    }

    let key_hash = hash_key(key);

    // Check cache first
    if let Some(cached) = cache.get_api_key(&key_hash).await {
        // Check expiry (MAX means no expiry)
        let now = chrono::Utc::now();
        if now > cached.expires_at {
            cache.invalidate_api_key(&key_hash).await;
            return Ok(None);
        }
        return Ok(Some(cached));
    }

    // Cache miss — go to DB
    let payload = validate_key(api_keys, users, key).await?;
    let Some(payload) = payload else {
        return Ok(None);
    };

    // Use MAX for keys without expiry so the cache expiry check always passes
    let expires_at = payload.expires_at.unwrap_or(DateTime::<Utc>::MAX_UTC);

    let verify = crate::auth::cache::VerifyPayload {
        user_id: payload.user_id,
        user_login: payload.user_name.unwrap_or_default(),
        role: payload.role,
        provider: "api-key".to_string(),
        group_id: payload.group_id,
        expires_at,
    };

    cache.insert_api_key(&key_hash, verify.clone()).await;
    Ok(Some(verify))
}

/// Revoke (delete) an API key. Only the owner can revoke.
/// Returns `Some(key_hash)` on success for cache invalidation, `None` if not found.
pub(crate) async fn revoke_key(
    api_keys: &dyn ApiKeyDao,
    key_id: &str,
    user_id: &str,
) -> Result<Option<String>, crate::auth::error::AuthError> {
    let row = api_keys.get_api_key_by_id(key_id).await?;

    let Some(row) = row else {
        debug!(key_id = %key_id, "API key not found for revocation");
        return Ok(None);
    };

    if row.user_id != user_id {
        warn!(key_id = %key_id, user_id = %user_id, key_owner = %row.user_id, "User tried to revoke another user's key");
        return Ok(None);
    }

    let key_hash = row.key_hash.clone();
    api_keys.delete_api_key(key_id).await?;
    info!(key_id = %key_id, user_id = %user_id, "Revoked API key");
    Ok(Some(key_hash))
}

/// List all API keys for a user.
pub(crate) async fn list_keys(
    api_keys: &dyn ApiKeyDao,
    user_id: &str,
) -> Result<Vec<ApiKey>, crate::auth::error::AuthError> {
    Ok(api_keys.list_api_keys_by_user(user_id).await?)
}

/// SHA-256 hash of a key.
pub fn hash_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}
