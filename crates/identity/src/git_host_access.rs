//! Git-host credential administration.

use std::sync::Arc;

use async_trait::async_trait;
use moatless_types::access_control::ScopeContext;
use moatless_core::git_provider::GitProviderError;
use moatless_core::secret_keys::GITHUB_TOKEN;

use crate::auth::credentials::{
    GITHUB_OAUTH_STASH_KEY, read_github_oauth_stash, stash_github_oauth_token,
};
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::{AUTH_METHOD_SECRET_REF, SECRET_REF_TOKEN_PLACEHOLDER};
use crate::secret::error::SecretError;
use moatless_types::secret::SecretKind;
use crate::secret::schemas::PutSecretRequest;
use crate::secret::service::SecretService;

#[derive(Clone, Debug)]
pub struct GitHostPrincipal {
    pub user_id: String,
    pub login: String,
    pub is_github_app_user: bool,
    pub scope_context: ScopeContext,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubProviderTokenStatus {
    pub connected: bool,
    pub login: Option<String>,
    pub token_type: Option<String>,
    pub has_pat_override: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitnessProviderTokenStatus {
    pub connected: bool,
    pub login: Option<String>,
    pub base_url: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum GitHostAccessError {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("Provider error: {0}")]
    Provider(#[source] GitProviderError),
    #[error("Secret storage error: {0}")]
    Secret(#[from] SecretError),
}

#[async_trait]
pub trait GitHostTokenValidator: Send + Sync {
    async fn validate_github(&self, token: &str) -> Result<String, GitProviderError>;
    async fn validate_gitness(
        &self,
        base_url: &str,
        token: &str,
    ) -> Result<String, GitProviderError>;
}

pub struct ProviderGitHostTokenValidator {
    client: reqwest::Client,
}

impl Default for ProviderGitHostTokenValidator {
    fn default() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl GitHostTokenValidator for ProviderGitHostTokenValidator {
    async fn validate_github(&self, token: &str) -> Result<String, GitProviderError> {
        moatless_core::github_client::validate_github_token(token).await
    }

    async fn validate_gitness(
        &self,
        base_url: &str,
        token: &str,
    ) -> Result<String, GitProviderError> {
        let base = base_url.trim_end_matches('/');
        let response = self
            .client
            .get(format!("{base}/api/v1/user"))
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
            .map_err(|source| GitProviderError::Unreachable {
                operation: "Gitness API",
                source,
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(GitProviderError::Rejected {
                operation: "Gitness API",
                status,
                body,
            });
        }

        let data: serde_json::Value =
            response
                .json()
                .await
                .map_err(|source| GitProviderError::UndecodableResponse {
                    operation: "Gitness",
                    source,
                })?;
        data.get("login")
            .or_else(|| data.get("uid"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                GitProviderError::MalformedResponse(
                    "Gitness response missing login/uid field".to_string(),
                )
            })
    }
}

pub struct GitHostAccess {
    provider_tokens: Arc<dyn ProviderTokenDao>,
    secrets: Arc<SecretService>,
    validator: Arc<dyn GitHostTokenValidator>,
}

impl GitHostAccess {
    pub fn new(
        provider_tokens: Arc<dyn ProviderTokenDao>,
        secrets: Arc<SecretService>,
        validator: Arc<dyn GitHostTokenValidator>,
    ) -> Self {
        Self {
            provider_tokens,
            secrets,
            validator,
        }
    }

    pub async fn github_status(
        &self,
        principal: &GitHostPrincipal,
    ) -> Result<GitHubProviderTokenStatus, GitHostAccessError> {
        let token = self
            .provider_tokens
            .find_provider_token_full(&principal.user_id, "github")
            .await?;
        let Some(row) = token else {
            return Ok(GitHubProviderTokenStatus {
                connected: false,
                login: None,
                token_type: None,
                has_pat_override: principal.is_github_app_user.then_some(false),
            });
        };
        let login = metadata_string(row.metadata.as_ref(), "login");
        Ok(GitHubProviderTokenStatus {
            connected: true,
            login,
            has_pat_override: principal
                .is_github_app_user
                .then_some(row.token_type.as_deref() == Some("pat")),
            token_type: row.token_type,
        })
    }

    pub async fn save_github_provider_token(
        &self,
        principal: &GitHostPrincipal,
        token: String,
    ) -> Result<GitHubProviderTokenStatus, GitHostAccessError> {
        let login = self
            .validator
            .validate_github(&token)
            .await
            .map_err(GitHostAccessError::Provider)?;
        let mut metadata = serde_json::json!({ "login": login });

        if principal.is_github_app_user {
            if let Some(existing) = self
                .provider_tokens
                .find_provider_token_full(&principal.user_id, "github")
                .await?
            {
                if existing.token_type.as_deref() == Some("pat") {
                    for key in ["oauth_login", "oauth_auth_method"] {
                        if let Some(value) = metadata_string(existing.metadata.as_ref(), key) {
                            metadata[key] = serde_json::Value::String(value);
                        }
                    }
                } else {
                    stash_github_oauth_token(
                        &self.secrets,
                        &principal.user_id,
                        &existing.access_token,
                    )
                    .await?;
                    if let Some(value) = metadata_string(existing.metadata.as_ref(), "login") {
                        metadata["oauth_login"] = serde_json::Value::String(value);
                    }
                    if let Some(value) = existing.auth_method {
                        metadata["oauth_auth_method"] = serde_json::Value::String(value);
                    }
                }
            }
        }

        self.secrets
            .put_secret(
                &principal.scope_context,
                &principal.user_id,
                PutSecretRequest {
                    scope: Some(moatless_api_schemas::enums::Scope::User),
                    key: GITHUB_TOKEN.to_string(),
                    kind: SecretKind::ProviderToken,
                    provider: Some("github".to_string()),
                    value: Some(token),
                    external_ref: None,
                },
            )
            .await?;
        self.provider_tokens
            .upsert_token_with_auth_method(
                &principal.user_id,
                "github",
                AUTH_METHOD_SECRET_REF,
                SECRET_REF_TOKEN_PLACEHOLDER,
                None,
                Some("pat"),
                Some(&metadata),
            )
            .await?;

        Ok(GitHubProviderTokenStatus {
            connected: true,
            login: Some(login),
            token_type: Some("pat".to_string()),
            has_pat_override: principal.is_github_app_user.then_some(true),
        })
    }

    pub async fn disconnect_github(
        &self,
        principal: &GitHostPrincipal,
    ) -> Result<GitHubProviderTokenStatus, GitHostAccessError> {
        self.provider_tokens
            .delete_token(&principal.user_id, "github")
            .await?;
        self.cleanup_github_secrets(&principal.user_id, "GitHub disconnect")
            .await;
        Ok(GitHubProviderTokenStatus {
            connected: false,
            login: None,
            token_type: None,
            has_pat_override: None,
        })
    }

    pub async fn restore_github_app_token(
        &self,
        principal: &GitHostPrincipal,
    ) -> Result<GitHubProviderTokenStatus, GitHostAccessError> {
        if !principal.is_github_app_user {
            return Err(GitHostAccessError::BadRequest(
                "PAT override removal is only available for GitHub App users".to_string(),
            ));
        }
        let token = self
            .provider_tokens
            .find_provider_token_full(&principal.user_id, "github")
            .await?
            .ok_or_else(|| GitHostAccessError::NotFound("No GitHub token found".to_string()))?;
        if token.token_type.as_deref() != Some("pat") {
            return Err(GitHostAccessError::BadRequest(
                "No PAT override to remove".to_string(),
            ));
        }
        let oauth_token = read_github_oauth_stash(&self.secrets, &principal.user_id)
            .await?
            .ok_or_else(|| {
                GitHostAccessError::BadRequest(
                    "Cannot restore GitHub App token — original token not preserved".to_string(),
                )
            })?;
        let login = metadata_string(token.metadata.as_ref(), "oauth_login")
            .unwrap_or_else(|| principal.login.clone());
        let restored_auth_method = metadata_string(token.metadata.as_ref(), "oauth_auth_method")
            .unwrap_or_else(|| "github_app".to_string());
        let metadata = serde_json::json!({ "login": login });

        self.provider_tokens
            .upsert_token_with_auth_method(
                &principal.user_id,
                "github",
                &restored_auth_method,
                &oauth_token,
                None,
                Some("bearer"),
                Some(&metadata),
            )
            .await?;
        self.cleanup_github_secrets(&principal.user_id, "PAT removal")
            .await;

        Ok(GitHubProviderTokenStatus {
            connected: true,
            login: Some(login),
            token_type: Some("bearer".to_string()),
            has_pat_override: Some(false),
        })
    }

    pub async fn gitness_status(
        &self,
        user_id: &str,
    ) -> Result<GitnessProviderTokenStatus, GitHostAccessError> {
        let token = self
            .provider_tokens
            .find_provider_token_full(user_id, "gitness")
            .await?;
        Ok(match token {
            Some(row) => GitnessProviderTokenStatus {
                connected: true,
                login: metadata_string(row.metadata.as_ref(), "login"),
                base_url: metadata_string(row.metadata.as_ref(), "base_url"),
            },
            None => GitnessProviderTokenStatus {
                connected: false,
                login: None,
                base_url: None,
            },
        })
    }

    pub async fn save_gitness_provider_token(
        &self,
        user_id: &str,
        base_url: String,
        token: String,
    ) -> Result<GitnessProviderTokenStatus, GitHostAccessError> {
        let login = self
            .validator
            .validate_gitness(&base_url, &token)
            .await
            .map_err(GitHostAccessError::Provider)?;
        let metadata = serde_json::json!({ "login": login, "base_url": base_url });
        self.provider_tokens
            .upsert_token(
                user_id,
                "gitness",
                &token,
                None,
                Some("pat"),
                Some(&metadata),
            )
            .await?;
        Ok(GitnessProviderTokenStatus {
            connected: true,
            login: Some(login),
            base_url: Some(base_url),
        })
    }

    pub async fn disconnect_gitness(
        &self,
        user_id: &str,
    ) -> Result<GitnessProviderTokenStatus, GitHostAccessError> {
        self.provider_tokens
            .delete_token(user_id, "gitness")
            .await?;
        Ok(GitnessProviderTokenStatus {
            connected: false,
            login: None,
            base_url: None,
        })
    }

    async fn cleanup_github_secrets(&self, user_id: &str, action: &str) {
        for (key, kind) in [
            (GITHUB_TOKEN, SecretKind::ProviderToken),
            (GITHUB_OAUTH_STASH_KEY, SecretKind::Internal),
        ] {
            if let Err(error) = self
                .secrets
                .delete_user_secret(user_id, key, Some(kind))
                .await
            {
                tracing::warn!(%error, user_id, key, action, "Failed to clean up GitHub secret");
            }
        }
    }
}

fn metadata_string(metadata: Option<&serde_json::Value>, key: &str) -> Option<String> {
    metadata
        .and_then(|value| value.get(key))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::Ordering;

    use super::*;
    use crate::auth::test_support::StubProviderTokens;
    use crate::secret::test_support::service_with;

    struct RejectingValidator;

    #[async_trait]
    impl GitHostTokenValidator for RejectingValidator {
        async fn validate_github(&self, _token: &str) -> Result<String, GitProviderError> {
            Err(GitProviderError::MalformedResponse(
                "invalid token".to_string(),
            ))
        }

        async fn validate_gitness(
            &self,
            _base_url: &str,
            _token: &str,
        ) -> Result<String, GitProviderError> {
            Err(GitProviderError::MalformedResponse(
                "invalid token".to_string(),
            ))
        }
    }

    fn access(tokens: Arc<StubProviderTokens>) -> GitHostAccess {
        GitHostAccess::new(
            tokens,
            Arc::new(service_with(Vec::new())),
            Arc::new(RejectingValidator),
        )
    }

    fn principal() -> GitHostPrincipal {
        GitHostPrincipal {
            user_id: "u1".to_string(),
            login: "user".to_string(),
            is_github_app_user: false,
            scope_context: ScopeContext {
                user_id: "u1".to_string(),
                is_admin: false,
            },
        }
    }

    #[tokio::test]
    async fn github_validation_failure_writes_nothing() {
        let tokens = Arc::new(StubProviderTokens::default());
        let result = access(Arc::clone(&tokens))
            .save_github_provider_token(&principal(), "bad".to_string())
            .await;

        assert!(matches!(result, Err(GitHostAccessError::Provider(_))));
        assert_eq!(tokens.writes.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn gitness_validation_failure_writes_nothing() {
        let tokens = Arc::new(StubProviderTokens::default());
        let result = access(Arc::clone(&tokens))
            .save_gitness_provider_token(
                "u1",
                "https://gitness.example".to_string(),
                "bad".to_string(),
            )
            .await;

        assert!(matches!(result, Err(GitHostAccessError::Provider(_))));
        assert_eq!(tokens.writes.load(Ordering::Relaxed), 0);
    }
}
