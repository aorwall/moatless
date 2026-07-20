//! Local credential row.

use chrono::{DateTime, Utc};

/// A row of `user_credentials`.
///
/// `Debug` is hand-written so the argon2 hash never reaches a log line.
#[derive(sqlx::FromRow, Clone)]
pub struct UserCredential {
    pub user_id: String,
    pub username: String,
    pub password_hash: String,
    pub must_change: bool,
    pub failed_attempts: i32,
    pub last_login_at: Option<DateTime<Utc>>,
    pub password_changed_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for UserCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UserCredential")
            .field("user_id", &self.user_id)
            .field("username", &self.username)
            .field("password_hash", &"[REDACTED]")
            .field("must_change", &self.must_change)
            .field("failed_attempts", &self.failed_attempts)
            .field("last_login_at", &self.last_login_at)
            .field("password_changed_at", &self.password_changed_at)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .finish()
    }
}
