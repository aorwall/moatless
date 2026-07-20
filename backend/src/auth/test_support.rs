//! Provider-token repository stub for unit tests.
//!
//! `ProviderTokenRepository` has no `automock` (mockall can't synthesise the
//! `Option<&str>` / `Option<&Value>` lifetimes in its async-trait methods), so
//! tests use this hand-written stub.

use async_trait::async_trait;

use super::models::{ProviderAccessToken, ProviderToken};
use super::provider_token_dao::ProviderTokenRepository;
use crate::credentials::CodexScope;

/// Returns one canned user-level row for every provider lookup; writes are
/// accepted and dropped.
pub struct StubProviderTokens(pub Option<ProviderToken>);

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

#[async_trait]
impl ProviderTokenRepository for StubProviderTokens {
    async fn find_provider_token(
        &self,
        _user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderAccessToken>, sqlx::Error> {
        Ok(self
            .0
            .as_ref()
            .filter(|r| r.provider == provider)
            .map(|r| ProviderAccessToken {
                access_token: r.access_token.clone(),
            }))
    }

    async fn find_provider_token_full(
        &self,
        _user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderToken>, sqlx::Error> {
        Ok(self.0.as_ref().filter(|r| r.provider == provider).cloned())
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
        Ok(())
    }

    async fn delete_token(&self, _user_id: &str, _provider: &str) -> Result<(), sqlx::Error> {
        Ok(())
    }

    async fn find_team_token(
        &self,
        _team_id: &str,
        _provider: &str,
    ) -> Result<Option<ProviderAccessToken>, sqlx::Error> {
        Ok(None)
    }

    async fn upsert_team_token(
        &self,
        _team_id: &str,
        _provider: &str,
        _auth_method: &str,
        _access_token: &str,
        _refresh_token: Option<&str>,
        _token_type: Option<&str>,
        _metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error> {
        Ok(())
    }

    async fn delete_team_token(&self, _team_id: &str, _provider: &str) -> Result<(), sqlx::Error> {
        Ok(())
    }

    async fn list_by_team(&self, _team_id: &str) -> Result<Vec<ProviderToken>, sqlx::Error> {
        Ok(vec![])
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
        Ok(())
    }

    async fn delete_codex_blob(&self, _scope: &CodexScope) -> Result<(), sqlx::Error> {
        Ok(())
    }
}
