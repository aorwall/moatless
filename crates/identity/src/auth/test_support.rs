//! Provider-token DAO stub for unit tests.
//!
//! `ProviderTokenDao` has no `automock` (mockall can't synthesise the
//! `Option<&str>` / `Option<&Value>` lifetimes in its async-trait methods), so
//! tests use this hand-written stub.

use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};

use super::models::{GithubTokenPresence, ProviderAccessToken, ProviderToken};
use super::provider_token_dao::ProviderTokenDao;
use crate::auth::codex_scope::CodexScope;
use crate::auth::github_scope::GitHubScope;

/// Canned `provider_tokens` rows at each scope. Writes are accepted and
/// dropped. Reads honour the same `user > global` precedence as
/// `PgProviderTokenDao`.
#[derive(Default)]
pub struct StubProviderTokens {
    /// Canned user-scope row, optionally restricted to [`Self::user_id`].
    pub user_row: Option<ProviderToken>,
    /// When set, the canned user row exists only at this exact user scope.
    pub user_id: Option<String>,
    /// The singleton global github row.
    pub global_row: Option<ProviderToken>,
    /// Number of persistence mutations attempted through this stub.
    pub writes: AtomicUsize,
}

impl StubProviderTokens {
    /// One canned user-level row for every provider lookup.
    pub fn with_user_row(row: Option<ProviderToken>) -> Self {
        Self {
            user_row: row,
            ..Self::default()
        }
    }

    /// One canned user-level row at an exact user scope.
    pub fn with_user_row_for(user_id: &str, row: ProviderToken) -> Self {
        Self {
            user_row: Some(row),
            user_id: Some(user_id.to_string()),
            ..Self::default()
        }
    }

    pub fn with_global_row(mut self, row: ProviderToken) -> Self {
        self.global_row = Some(row);
        self
    }

    fn user_row(&self, user_id: &str) -> Option<&ProviderToken> {
        self.user_row
            .as_ref()
            .filter(|_| self.user_id.as_deref().is_none_or(|id| id == user_id))
    }

    fn github_user_row(&self, user_id: &str) -> Option<&ProviderToken> {
        self.user_row(user_id).filter(|r| r.provider == "github")
    }
}

/// Build a GitHub `provider_tokens` row with the given `auth_method` and
/// `access_token`.
pub fn github_row(auth_method: &str, access_token: &str) -> ProviderToken {
    ProviderToken {
        user_id: "u1".to_string(),
        provider: "github".to_string(),
        access_token: access_token.to_string(),
        refresh_token: None,
        token_type: Some("pat".to_string()),
        auth_method: Some(auth_method.to_string()),
        expires_at: None,
        metadata: None,
        created_at: None,
        updated_at: None,
        provider_user_id: None,
        provider_login: None,
    }
}

fn presence(scope: &GitHubScope, row: &ProviderToken) -> GithubTokenPresence {
    GithubTokenPresence {
        scope: scope.scope_kind().to_string(),
        user_id: match scope {
            GitHubScope::User(id) => Some(id.clone()),
            _ => None,
        },
        team_id: None,
        auth_method: row.auth_method.clone(),
        login: row
            .metadata
            .as_ref()
            .and_then(|m| m.get("login"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        refresh_failure: row
            .metadata
            .as_ref()
            .is_some_and(|m| m.get("refresh_failure").is_some()),
    }
}

#[async_trait]
impl ProviderTokenDao for StubProviderTokens {
    async fn find_provider_token(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderAccessToken>, sqlx::Error> {
        Ok(self
            .user_row(user_id)
            .filter(|r| r.provider == provider)
            .map(|r| ProviderAccessToken {
                access_token: r.access_token.clone(),
            }))
    }

    async fn find_provider_token_full(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderToken>, sqlx::Error> {
        Ok(self
            .user_row(user_id)
            .filter(|r| r.provider == provider)
            .cloned())
    }

    async fn upsert_token(
        &self,
        _user_id: &str,
        _provider: &str,
        _access_token: &str,
        _refresh_token: Option<&str>,
        _token_type: Option<&str>,
        _metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn upsert_token_with_auth_method(
        &self,
        _user_id: &str,
        _provider: &str,
        _auth_method: &str,
        _access_token: &str,
        _refresh_token: Option<&str>,
        _token_type: Option<&str>,
        _metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn delete_token(&self, _user_id: &str, _provider: &str) -> Result<(), sqlx::Error> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn find_codex_blob(
        &self,
        _scope: &CodexScope,
    ) -> Result<Option<ProviderToken>, sqlx::Error> {
        Ok(None)
    }

    async fn upsert_codex_blob(
        &self,
        _scope: &CodexScope,
        _access_token: &str,
        _refresh_token: Option<&str>,
        _token_type: Option<&str>,
        _metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn delete_codex_blob(&self, _scope: &CodexScope) -> Result<(), sqlx::Error> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn find_github_row(
        &self,
        scope: &GitHubScope,
    ) -> Result<Option<ProviderToken>, sqlx::Error> {
        Ok(match scope {
            GitHubScope::User(user_id) => self.github_user_row(user_id).cloned(),
            GitHubScope::Global => self.global_row.clone(),
        })
    }

    async fn upsert_github_row(
        &self,
        _scope: &GitHubScope,
        _auth_method: &str,
        _access_token: &str,
        _token_type: Option<&str>,
        _metadata: Option<&serde_json::Value>,
        _provider_user_id: Option<&str>,
        _provider_login: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn delete_github_row(&self, _scope: &GitHubScope) -> Result<(), sqlx::Error> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn find_github_scope_presence(
        &self,
        user_id: &str,
    ) -> Result<Option<GithubTokenPresence>, sqlx::Error> {
        if let Some(row) = self.github_user_row(user_id) {
            return Ok(Some(presence(&GitHubScope::user(user_id), row)));
        }
        Ok(self
            .global_row
            .as_ref()
            .map(|row| presence(&GitHubScope::Global, row)))
    }
}
