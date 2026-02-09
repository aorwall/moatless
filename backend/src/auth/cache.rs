//! In-memory cache for auth verification (sessions + API keys).
//!
//! Eliminates ~97% of DB hits on the hot `verify_handler` path by caching
//! the minimal payload needed for ForwardAuth responses.

use chrono::NaiveDateTime;
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;

/// Minimal payload cached for verify responses.
/// Contains only what `verify_handler` needs — no tokens are cached.
#[derive(Debug, Clone)]
pub struct VerifyPayload {
    pub user_id: String,
    pub user_login: String,
    pub role: String,
    pub provider: String,
    pub group_id: Option<String>,
    pub expires_at: NaiveDateTime,
}

/// Two-cache auth verifier: one for sessions, one for API keys.
#[derive(Clone)]
pub struct AuthCache {
    sessions: Arc<Cache<String, VerifyPayload>>,
    api_keys: Arc<Cache<String, VerifyPayload>>,
}

impl AuthCache {
    /// Create a new auth cache with the given TTL in seconds.
    pub fn new(ttl_secs: u64) -> Self {
        let ttl = Duration::from_secs(ttl_secs);

        let sessions = Cache::builder()
            .max_capacity(10_000)
            .time_to_live(ttl)
            .build();

        let api_keys = Cache::builder()
            .max_capacity(1_000)
            .time_to_live(ttl)
            .build();

        Self {
            sessions: Arc::new(sessions),
            api_keys: Arc::new(api_keys),
        }
    }

    // -- Session cache --

    pub async fn get_session(&self, session_id: &str) -> Option<VerifyPayload> {
        self.sessions.get(session_id).await
    }

    pub async fn insert_session(&self, session_id: &str, payload: VerifyPayload) {
        self.sessions.insert(session_id.to_string(), payload).await;
    }

    pub async fn invalidate_session(&self, session_id: &str) {
        self.sessions.invalidate(session_id).await;
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
