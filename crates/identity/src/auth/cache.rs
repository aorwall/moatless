//! In-memory cache for auth verification (API keys).
//!
//! Session authentication uses self-validating JWTs — no cache needed.
//! This cache eliminates DB hits on the API key verification path.

use chrono::{DateTime, Utc};
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;

use moatless_types::user::UserRole;

/// Minimal payload cached for verify responses.
/// Contains only what `verify_handler` needs — no tokens are cached.
#[derive(Debug, Clone)]
pub struct VerifyPayload {
    pub user_id: String,
    pub user_login: String,
    pub role: UserRole,
    pub provider: String,
    pub group_id: Option<String>,
    pub expires_at: DateTime<Utc>,
}

/// Auth cache for API key verification.
#[derive(Clone)]
pub struct AuthCache {
    api_keys: Arc<Cache<String, VerifyPayload>>,
}

impl AuthCache {
    /// Create a new auth cache with the given TTL in seconds.
    pub fn new(ttl_secs: u64) -> Self {
        let ttl = Duration::from_secs(ttl_secs);

        let api_keys = Cache::builder()
            .max_capacity(1_000)
            .time_to_live(ttl)
            .build();

        Self {
            api_keys: Arc::new(api_keys),
        }
    }

    // -- API key cache --

    pub async fn get_api_key(&self, key_hash: &str) -> Option<VerifyPayload> {
        self.api_keys.get(key_hash).await
    }

    pub async fn insert_api_key(&self, key_hash: &str, payload: VerifyPayload) {
        self.api_keys.insert(key_hash.to_string(), payload).await;
    }

    pub async fn invalidate_api_key(&self, key_hash: &str) {
        self.api_keys.invalidate(key_hash).await;
    }
}
