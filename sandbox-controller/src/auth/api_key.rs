//! API key service — generate, validate, revoke.

use chrono::NaiveDateTime;
use rand::Rng;
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

use crate::db::api_keys::{self, ApiKeyRow};
use crate::db::users;
use crate::db::DbPool;

const KEY_PREFIX: &str = "mvk_";
const KEY_BYTES: usize = 32;

/// Payload extracted from a validated API key.
pub struct ApiKeyPayload {
    pub key_id: String,
    pub user_id: String,
    pub user_name: Option<String>,
    pub scopes: Option<Vec<String>>,
    pub group_id: Option<String>,
    pub expires_at: Option<NaiveDateTime>,
}

/// Result of creating a new API key.
#[derive(serde::Serialize)]
pub struct ApiKeyCreateResult {
    pub id: String,
    pub key: String,
    pub name: String,
    pub key_prefix: String,
    pub scopes: Option<Vec<String>>,
    pub expires_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
}

/// Generate a new API key for a user.
pub async fn generate_key(
    pool: &DbPool,
    user_id: &str,
    name: &str,
    scopes: Option<Vec<String>>,
    expires_at: Option<NaiveDateTime>,
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

    let now = chrono::Utc::now().naive_utc();

    let row = ApiKeyRow {
        id: id.clone(),
        user_id: user_id.to_string(),
        name: name.to_string(),
        key_hash,
        key_prefix: key_prefix.clone(),
        scopes: scopes.as_ref().map(|s| serde_json::to_string(s).unwrap_or_default()),
        group_id: group_id.map(String::from),
        last_used_at: None,
        expires_at,
        created_at: now,
    };
    api_keys::insert_api_key(pool, &row).await?;

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
pub async fn validate_key(
    pool: &DbPool,
    key: &str,
) -> Result<Option<ApiKeyPayload>, crate::auth::error::AuthError> {
    if !key.starts_with(KEY_PREFIX) {
        return Ok(None);
    }

    let key_hash = hash_key(key);
    let row = api_keys::get_api_key_by_hash(pool, &key_hash).await?;

    let Some(row) = row else {
        debug!(key_prefix = &key[..12], "API key not found");
        return Ok(None);
    };

    // Check expiration
    if let Some(exp) = row.expires_at {
        if chrono::Utc::now().naive_utc() > exp {
            debug!(key_id = %row.id, "API key expired");
            return Ok(None);
        }
    }

    // Update last_used_at (fire and forget)
    let pool_clone = pool.clone();
    let key_id_clone = row.id.clone();
    tokio::spawn(async move {
        if let Err(e) = api_keys::update_last_used(&pool_clone, &key_id_clone).await {
            warn!(error = %e, key_id = %key_id_clone, "Failed to update API key last used");
        }
    });

    // Get user info
    let user = users::get_user(pool, &row.user_id).await?;

    Ok(Some(ApiKeyPayload {
        key_id: row.id,
        user_id: row.user_id,
        user_name: user.map(|u| u.login),
        scopes: row.scopes.and_then(|s| serde_json::from_str(&s).ok()),
        group_id: row.group_id,
        expires_at: row.expires_at,
    }))
}

/// Revoke (delete) an API key. Only the owner can revoke.
pub async fn revoke_key(
    pool: &DbPool,
    key_id: &str,
    user_id: &str,
) -> Result<bool, crate::auth::error::AuthError> {
    let row = api_keys::get_api_key_by_id(pool, key_id).await?;

    let Some(row) = row else {
        debug!(key_id = %key_id, "API key not found for revocation");
        return Ok(false);
    };

    if row.user_id != user_id {
        warn!(key_id = %key_id, user_id = %user_id, key_owner = %row.user_id, "User tried to revoke another user's key");
        return Ok(false);
    }

    api_keys::delete_api_key(pool, key_id).await?;
    info!(key_id = %key_id, user_id = %user_id, "Revoked API key");
    Ok(true)
}

/// List all API keys for a user.
pub async fn list_keys(
    pool: &DbPool,
    user_id: &str,
) -> Result<Vec<ApiKeyRow>, crate::auth::error::AuthError> {
    Ok(api_keys::list_api_keys_by_user(pool, user_id).await?)
}

/// SHA-256 hash of a key.
fn hash_key(key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    hex::encode(hasher.finalize())
}

