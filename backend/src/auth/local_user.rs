//! Auto-create the `local-user` in dev mode (no auth provider).
//!
//! This ensures the default dev-mode identity has a matching DB row so that
//! user-scoped features (settings, team membership, etc.) work out of the box.

use chrono::Utc;
use tracing::{info, warn};

use super::models::UserRole;
use super::users::{User, UserRepository};

const LOCAL_USER_ID: &str = "local-user";
const LOCAL_USER_LOGIN: &str = "local-user";
const LOCAL_USER_PROVIDER: &str = "none";

/// Ensure the `local-user` row exists. Idempotent — skips if already present.
pub async fn ensure_local_user(users: &dyn UserRepository) {
    match users.get_user(LOCAL_USER_ID).await {
        Ok(Some(_)) => {
            info!("Dev-mode local-user already exists");
            return;
        }
        Ok(None) => { /* proceed to create */ }
        Err(e) => {
            warn!(error = %e, "Failed to check for local-user");
            return;
        }
    }

    let now = Utc::now();
    let user = User {
        id: LOCAL_USER_ID.to_string(),
        login: LOCAL_USER_LOGIN.to_string(),
        email: None,
        name: Some("Local User".to_string()),
        avatar_url: None,
        provider: LOCAL_USER_PROVIDER.to_string(),
        role: UserRole::Admin,
        created_at: now,
        last_login_at: now,
        settings: serde_json::json!({}),
        is_bot: false,
    };

    if let Err(e) = users.upsert_user(&user).await {
        warn!(error = %e, "Failed to create local-user");
        return;
    }

    info!("Dev-mode local-user created");
}
