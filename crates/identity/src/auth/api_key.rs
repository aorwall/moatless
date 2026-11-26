//! API key service — generate, validate, revoke.

use std::sync::Arc;

use chrono::{DateTime, Utc};
use rand::RngExt;
use sha2::{Digest, Sha256};
use tracing::{debug, info, warn};

use super::api_keys::ApiKey;
use super::api_keys_dao::ApiKeyDao;
use super::users_dao::UserDao;
use moatless_types::user::{UserRole, UserStatus};

const KEY_PREFIX: &str = "mvk_";
const KEY_BYTES: usize = 32;

/// Payload extracted from a validated API key.
pub struct ApiKeyPayload {
    pub user_id: String,
    pub user_name: Option<String>,
    pub role: UserRole,
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
///
/// `idle_ttl` makes `expires_at` slide: see [`ApiKey::idle_ttl_secs`].
pub(crate) async fn generate_key(
    api_keys: &dyn ApiKeyDao,
    user_id: &str,
    name: &str,
    scopes: Option<Vec<String>>,
    expires_at: Option<DateTime<Utc>>,
    idle_ttl: Option<chrono::Duration>,
) -> Result<ApiKeyCreateResult, crate::auth::error::AuthError> {
    let key_bytes: [u8; KEY_BYTES] = rand::rng().random();
    let key_hex = hex::encode(key_bytes);
    let full_key = format!("{KEY_PREFIX}{key_hex}");

    let key_hash = hash_key(&full_key);
    let key_prefix = format!("{}{}", KEY_PREFIX, &key_hex[..8]);

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
        last_used_at: None,
        expires_at,
        idle_ttl_secs: idle_ttl.map(|ttl| ttl.num_seconds()),
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

    let now = chrono::Utc::now();
    if let Some(exp) = row.expires_at {
        if now > exp {
            debug!(key_id = %row.id, "API key expired");
            return Ok(None);
        }
    }
    let renewed_expires_at = renewed_expiry(&row, now);

    // Update last_used_at (fire and forget)
    let api_keys_clone = api_keys.clone();
    let key_id_clone = row.id.clone();
    tokio::spawn(async move {
        if let Err(e) = api_keys_clone
            .update_last_used(&key_id_clone, renewed_expires_at)
            .await
        {
            warn!(error = %e, key_id = %key_id_clone, "Failed to update API key last used");
        }
    });

    // Get user info. The stored id is the uuid's text form.
    let user = match row.user_id.parse::<uuid::Uuid>() {
        Ok(uid) => users.get_user(&uid).await?,
        Err(_) => None,
    };
    let (user_name, role) = match user {
        // A disabled account's key stops working, like its sessions.
        Some(u) if u.status == UserStatus::Disabled => {
            debug!(user_id = %u.id, key_id = %row.id, "API key belongs to a disabled account");
            return Ok(None);
        }
        Some(u) => (Some(u.login), u.role),
        None => (None, UserRole::User),
    };

    Ok(Some(ApiKeyPayload {
        user_id: row.user_id,
        user_name,
        role,
        // The renewed instant, so a cached entry does not expire on the old one.
        expires_at: renewed_expires_at.or(row.expires_at),
    }))
}

/// The expiry a use of `row` at `now` moves it to, if it slides at all.
/// Never earlier than it already was.
fn renewed_expiry(row: &ApiKey, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let renewed = now + chrono::Duration::seconds(row.idle_ttl_secs?);
    Some(
        row.expires_at
            .map_or(renewed, |current| current.max(renewed)),
    )
}

/// Validates an API key with cache-aside — the path `auth_middleware` and
/// `verify_handler` take for every `Bearer mvk_` request.
///
/// A cache hit skips [`validate_key`], and with it the `status` check, until
/// the entry ages out. See `docs/deferred-changes.md`.
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

    if let Some(cached) = cache.get_api_key(&key_hash).await {
        // Check expiry (MAX means no expiry)
        let now = chrono::Utc::now();
        if now > cached.expires_at {
            cache.invalidate_api_key(&key_hash).await;
            return Ok(None);
        }
        return Ok(Some(cached));
    }

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
        expires_at,
    };

    cache.insert_api_key(&key_hash, verify.clone()).await;
    Ok(Some(verify))
}

/// Revoke (delete) an API key. Only the owner can revoke. Returns
/// `Some(key_hash)` on success for cache invalidation, `None` if not found.
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn key(expires_at: Option<DateTime<Utc>>, idle_ttl_secs: Option<i64>) -> ApiKey {
        ApiKey {
            id: "k".to_string(),
            user_id: "u".to_string(),
            name: "n".to_string(),
            key_hash: "h".to_string(),
            key_prefix: "mvk_00000000".to_string(),
            scopes: None,
            last_used_at: None,
            expires_at,
            idle_ttl_secs,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn a_key_with_an_idle_ttl_is_renewed_from_now() {
        let now = Utc::now();
        let row = key(Some(now + Duration::days(1)), Some(90 * 86_400));
        assert_eq!(renewed_expiry(&row, now), Some(now + Duration::days(90)));
    }

    #[test]
    fn renewal_never_shortens_a_key() {
        let now = Utc::now();
        let later = now + Duration::days(200);
        let row = key(Some(later), Some(86_400));
        assert_eq!(renewed_expiry(&row, now), Some(later));
    }

    #[test]
    fn a_key_without_an_idle_ttl_keeps_its_expiry() {
        let now = Utc::now();
        assert_eq!(
            renewed_expiry(&key(Some(now + Duration::days(7)), None), now),
            None
        );
        assert_eq!(renewed_expiry(&key(None, None), now), None);
    }
}
