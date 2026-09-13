//! The policy half of a Codex blob refresh.
//!
//! The store owns the row, the advisory lock, the compare-and-swap and the
//! failure marker; the caller owns the staleness rule and the network call.

use async_trait::async_trait;

use moatless_types::agent::CodexScope;

/// What the policy half decided to do with a Codex blob it was handed.
pub enum CodexBlobRefresh {
    /// The stored blob is already good. Write nothing.
    Fresh,
    /// Replace the blob, and the refresh token beside it when one rotated.
    Rotated {
        blob: String,
        refresh_token: Option<String>,
    },
    /// The refresh failed for good. Record `reason` on the row.
    Permanent { reason: String },
    /// The refresh failed for now. Keep the stored blob.
    Transient,
    /// The stored blob could not be read at all.
    Unreadable,
}

/// Is this blob stale, and what replaces it.
#[async_trait]
pub trait CodexBlobRefresher: Send + Sync {
    async fn refresh(&self, blob: &str) -> CodexBlobRefresh;
}

/// What a
/// [`crate::auth::provider_token_dao::ProviderTokenDao::refresh_codex_blob`]
/// left on the row.
pub enum CodexBlobOutcome {
    /// The stored blob, unchanged — already fresh, a transient failure, or
    /// another replica held the lock and is doing the work.
    Unchanged { blob: String },
    /// The blob was replaced and the new value is persisted.
    Rotated { blob: String },
    /// The refresh failed permanently. `reason` is now recorded on the row;
    /// `was_already_failing` is true when the row already carried the same
    /// reason, so a caller can log the transition rather than every retry.
    PermanentFailure {
        reason: String,
        was_already_failing: bool,
    },
    /// The stored blob could not be read.
    Unreadable { blob: String },
}

/// Stable key shared by the advisory lock and in-process credential mutex.
pub fn codex_lock_key(scope: &CodexScope) -> String {
    match scope {
        CodexScope::User(id) => format!("codex-refresh:user:{id}"),
        CodexScope::Global => "codex-refresh:global".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lock_key_is_distinct_across_scopes_with_same_id() {
        let user = codex_lock_key(&CodexScope::user("foo"));
        let global = codex_lock_key(&CodexScope::Global);
        assert_eq!(user, "codex-refresh:user:foo");
        assert_eq!(global, "codex-refresh:global");
        assert_ne!(user, global);
        assert!(user.contains("user"));
        assert!(global.contains("global"));
    }
}
