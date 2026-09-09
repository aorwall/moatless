//! Credential fixtures a consumer's tests build a [`CredentialService`] from.
//!
//! The `provider_tokens` store is private to this crate, so a peer band cannot
//! write its own fake. [`InMemoryTokens`] is the seedable one it takes instead.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use tokio::sync::Mutex;

use moatless_types::agent::CodexScope;

use crate::auth::codex_blob::{CodexBlobOutcome, CodexBlobRefresher};
use crate::auth::models::{GithubTokenPresence, ProviderAccessToken, ProviderToken};
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::test_support::StubProviderTokens;
use crate::credentials::CredentialService;

/// Seedable provider-token rows held in memory. Codex refresh is inert: a
/// service built over this has no HTTP client, so `refresh_codex_locked` falls
/// back to the raw read of [`Self::with_codex_blob`].
#[derive(Default)]
pub struct InMemoryTokens {
    tokens: Mutex<HashMap<(String, String), String>>,
    codex_blob: Option<String>,
    upsert_count: AtomicUsize,
}

impl InMemoryTokens {
    /// No rows at any scope.
    pub fn empty() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// One canned Codex `auth.json` for every scope.
    pub fn with_codex_blob(blob: &str) -> Arc<Self> {
        Arc::new(Self {
            codex_blob: Some(blob.to_string()),
            ..Self::default()
        })
    }

    /// One canned provider row, which drives the dedup path of
    /// [`CredentialService::store`].
    pub fn with_token(user_id: &str, provider: &str, value: &str) -> Arc<Self> {
        Arc::new(Self {
            tokens: Mutex::new(HashMap::from([(
                (user_id.to_string(), provider.to_string()),
                value.to_string(),
            )])),
            ..Self::default()
        })
    }

    /// How many upserts reached the store, so a test tells a persist from a
    /// dedup hit.
    pub fn upsert_count(&self) -> usize {
        self.upsert_count.load(Ordering::SeqCst)
    }

    /// The stored value at `(user_id, provider)`.
    pub async fn token(&self, user_id: &str, provider: &str) -> Option<String> {
        self.tokens
            .lock()
            .await
            .get(&(user_id.to_string(), provider.to_string()))
            .cloned()
    }
}

/// A credential service over seeded in-memory rows.
pub fn credentials_over(tokens: Arc<InMemoryTokens>) -> Arc<CredentialService> {
    Arc::new(CredentialService::new(tokens))
}

/// A credential service over the canned-row stub, whose `writes` counter a test
/// asserts persistence ordering on.
pub fn credentials_over_stub(tokens: Arc<StubProviderTokens>) -> Arc<CredentialService> {
    Arc::new(CredentialService::new(tokens))
}

fn codex_row(access_token: String) -> ProviderToken {
    ProviderToken {
        user_id: "u1".to_string(),
        provider: "codex".to_string(),
        access_token,
        refresh_token: None,
        token_type: None,
        auth_method: None,
        expires_at: None,
        metadata: None,
        created_at: None,
        updated_at: None,
        provider_user_id: None,
        provider_login: None,
    }
}

#[async_trait]
impl ProviderTokenDao for InMemoryTokens {
    async fn find_user_by_provider_login(
        &self,
        _provider: &str,
        _provider_login: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        Ok(None)
    }

    async fn healthy_codex_scopes(&self) -> Result<Vec<CodexScope>, sqlx::Error> {
        Ok(Vec::new())
    }

    async fn find_user_by_provider_account(
        &self,
        _provider: &str,
        _provider_user_id: &str,
    ) -> Result<Option<String>, sqlx::Error> {
        Ok(None)
    }

    async fn find_app_token(&self, _provider: &str) -> Result<Option<ProviderToken>, sqlx::Error> {
        Ok(None)
    }

    async fn upsert_app_token(
        &self,
        _provider: &str,
        _access_token: &str,
        _refresh_token: Option<&str>,
        _token_type: Option<&str>,
        _metadata: Option<&serde_json::Value>,
        _expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<(), sqlx::Error> {
        Ok(())
    }

    async fn refresh_codex_blob(
        &self,
        _scope: &CodexScope,
        _refresher: &(dyn CodexBlobRefresher + '_),
    ) -> Result<Option<CodexBlobOutcome>, sqlx::Error> {
        Ok(None)
    }

    async fn find_provider_token(
        &self,
        user_id: &str,
        provider: &str,
    ) -> Result<Option<ProviderAccessToken>, sqlx::Error> {
        Ok(self
            .token(user_id, provider)
            .await
            .map(|access_token| ProviderAccessToken { access_token }))
    }

    async fn find_provider_token_full(
        &self,
        _user_id: &str,
        _provider: &str,
    ) -> Result<Option<ProviderToken>, sqlx::Error> {
        Ok(None)
    }

    async fn upsert_token(
        &self,
        user_id: &str,
        provider: &str,
        access_token: &str,
        _refresh_token: Option<&str>,
        _token_type: Option<&str>,
        _metadata: Option<&serde_json::Value>,
    ) -> Result<(), sqlx::Error> {
        self.upsert_count.fetch_add(1, Ordering::SeqCst);
        self.tokens.lock().await.insert(
            (user_id.to_string(), provider.to_string()),
            access_token.to_string(),
        );
        Ok(())
    }

    async fn upsert_token_with_auth_method(
        &self,
        user_id: &str,
        provider: &str,
        _auth_method: &str,
        access_token: &str,
        refresh_token: Option<&str>,
        token_type: Option<&str>,
        metadata: Option<&serde_json::Value>,
        _expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<(), sqlx::Error> {
        self.upsert_token(
            user_id,
            provider,
            access_token,
            refresh_token,
            token_type,
            metadata,
        )
        .await
    }

    async fn delete_token(&self, _user_id: &str, _provider: &str) -> Result<(), sqlx::Error> {
        Ok(())
    }

    async fn find_codex_blob(
        &self,
        _scope: &CodexScope,
    ) -> Result<Option<ProviderToken>, sqlx::Error> {
        Ok(self.codex_blob.clone().map(codex_row))
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

    async fn find_github_row(&self, _user_id: &str) -> Result<Option<ProviderToken>, sqlx::Error> {
        Ok(None)
    }

    async fn find_github_scope_presence(
        &self,
        _user_id: &str,
    ) -> Result<Option<GithubTokenPresence>, sqlx::Error> {
        Ok(None)
    }
}
