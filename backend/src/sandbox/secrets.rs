//! Task credential resolution for sandbox runtime consumers.

use std::collections::HashMap;
use std::sync::Arc;

use tracing::warn;

use crate::auth::github_app_service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenRepository;
use crate::auth::provider_tokens::ProviderTokenAuthRepository;
use crate::auth::token_refresh::ensure_fresh_token;
use crate::secret::models::is_reserved_key;
use crate::secret::service::SecretService;
use crate::task::dao::TaskRepository;

pub const GITHUB_TOKEN: &str = "GITHUB_TOKEN";
pub const GITLAB_TOKEN: &str = "GITLAB_TOKEN";
pub const GITNESS_TOKEN: &str = "GITNESS_TOKEN";
pub const CODEX_AUTH_JSON: &str = "CODEX_AUTH_JSON";
pub const OPENAI_API_KEY: &str = "OPENAI_API_KEY";
pub const ANTHROPIC_API_KEY: &str = "ANTHROPIC_API_KEY";
pub const CLAUDE_CODE_OAUTH_TOKEN: &str = "CLAUDE_CODE_OAUTH_TOKEN";

pub struct TaskCredentialInputs<'a> {
    pub task_id: &'a str,
    pub user_id: &'a str,
    pub team_id: Option<&'a str>,
    pub secrets: Option<&'a Arc<SecretService>>,
    pub provider_tokens: Option<&'a Arc<dyn ProviderTokenRepository>>,
    pub github_app_service: Option<&'a Arc<GitHubAppService>>,
    pub provider_token_auth: Option<&'a Arc<dyn ProviderTokenAuthRepository>>,
}

pub async fn resolve_task_credentials(input: TaskCredentialInputs<'_>) -> HashMap<String, String> {
    let mut credentials = resolve_scoped_secrets(
        input.secrets,
        input.user_id,
        input.team_id,
        input.task_id,
    )
    .await;

    fill_provider_tokens(
        &mut credentials,
        input.provider_tokens,
        input.team_id,
        input.user_id,
        input.github_app_service,
        input.provider_token_auth,
    )
    .await;

    credentials
}

/// Resolve the shared env delivered to every server sidecar.
///
/// Returns the full accessible scope chain (global > team > user) with
/// reserved runtime keys filtered out. The agent env is a superset that
/// additionally includes provider tokens.
pub async fn resolve_server_env(
    user_id: &str,
    team_id: Option<&str>,
    secrets: Option<&Arc<SecretService>>,
) -> HashMap<String, String> {
    let Some(secrets) = secrets else {
        return HashMap::new();
    };
    match secrets.resolve_accessible_plaintext(user_id, team_id).await {
        Ok(mut env) => {
            env.retain(|key, _| !is_reserved_key(key));
            env
        }
        Err(e) => {
            warn!(user_id, error = %e, "Failed to resolve server env secrets");
            HashMap::new()
        }
    }
}

pub(crate) async fn resolve_task_team_id(
    tasks: Option<&Arc<dyn TaskRepository>>,
    task_id: &str,
) -> Option<String> {
    let tasks = tasks?;
    match tasks.find_by_id_internal(task_id).await {
        Ok(Some(task)) => task.team_id,
        Ok(None) => {
            warn!(task_id, "resolve_task_team_id: task not found");
            None
        }
        Err(e) => {
            warn!(task_id, error = %e, "resolve_task_team_id: DB error");
            None
        }
    }
}

async fn resolve_scoped_secrets(
    secrets: Option<&Arc<SecretService>>,
    user_id: &str,
    team_id: Option<&str>,
    task_id: &str,
) -> HashMap<String, String> {
    let Some(secrets) = secrets else {
        return HashMap::new();
    };

    match secrets.resolve_accessible_plaintext(user_id, team_id).await {
        Ok(credentials) => credentials,
        Err(e) => {
            warn!(task_id, error = %e, "Failed to resolve scoped secrets");
            HashMap::new()
        }
    }
}

async fn fill_provider_tokens(
    credentials: &mut HashMap<String, String>,
    provider_tokens: Option<&Arc<dyn ProviderTokenRepository>>,
    team_id: Option<&str>,
    user_id: &str,
    github_app_service: Option<&Arc<GitHubAppService>>,
    provider_token_auth: Option<&Arc<dyn ProviderTokenAuthRepository>>,
) {
    let Some(provider_tokens) = provider_tokens else {
        return;
    };

    if let Some(team_id) = team_id {
        fill_team_provider_tokens(credentials, provider_tokens, team_id).await;
    }
    fill_user_provider_tokens(
        credentials,
        provider_tokens,
        user_id,
        github_app_service,
        provider_token_auth,
    )
    .await;
}

async fn fill_team_provider_tokens(
    credentials: &mut HashMap<String, String>,
    provider_tokens: &Arc<dyn ProviderTokenRepository>,
    team_id: &str,
) {
    for (provider, cred_key) in [
        ("github", GITHUB_TOKEN),
        ("gitlab", GITLAB_TOKEN),
        ("gitness", GITNESS_TOKEN),
        ("codex", CODEX_AUTH_JSON),
        ("openai", OPENAI_API_KEY),
    ] {
        if credentials.contains_key(cred_key) {
            continue;
        }
        match provider_tokens.find_team_token(team_id, provider).await {
            Ok(Some(row)) => {
                credentials.insert(cred_key.to_string(), row.access_token);
            }
            Ok(None) => {}
            Err(e) => {
                warn!(error = %e, team_id, provider, "Failed to query team provider_token");
            }
        }
    }

    if credentials.contains_key(CLAUDE_CODE_OAUTH_TOKEN)
        || credentials.contains_key(ANTHROPIC_API_KEY)
    {
        return;
    }
    match provider_tokens.find_team_token(team_id, "claude").await {
        Ok(Some(row)) => {
            credentials.insert(CLAUDE_CODE_OAUTH_TOKEN.to_string(), row.access_token);
        }
        Ok(None) => {}
        Err(e) => {
            warn!(error = %e, team_id, "Failed to query team claude provider_token");
        }
    }
}

async fn fill_user_provider_tokens(
    credentials: &mut HashMap<String, String>,
    provider_tokens: &Arc<dyn ProviderTokenRepository>,
    user_id: &str,
    github_app_service: Option<&Arc<GitHubAppService>>,
    provider_token_auth: Option<&Arc<dyn ProviderTokenAuthRepository>>,
) {
    if !credentials.contains_key(GITHUB_TOKEN) {
        let resolved = if let (Some(github_app), Some(token_auth)) =
            (github_app_service, provider_token_auth)
        {
            match ensure_fresh_token(
                provider_tokens.as_ref(),
                token_auth.as_ref(),
                github_app,
                user_id,
            )
            .await
            {
                Ok(token) => Some(token),
                Err(e) => {
                    warn!(
                        error = %e,
                        user_id,
                        "GitHub App token refresh failed, falling back to raw token"
                    );
                    provider_tokens
                        .find_provider_token(user_id, "github")
                        .await
                        .ok()
                        .flatten()
                        .map(|row| row.access_token)
                }
            }
        } else {
            provider_tokens
                .find_provider_token(user_id, "github")
                .await
                .ok()
                .flatten()
                .map(|row| row.access_token)
        };

        if let Some(token) = resolved {
            credentials.insert(GITHUB_TOKEN.to_string(), token);
        }
    }

    for (provider, cred_key) in [
        ("gitlab", GITLAB_TOKEN),
        ("gitness", GITNESS_TOKEN),
        ("codex", CODEX_AUTH_JSON),
        ("openai", OPENAI_API_KEY),
    ] {
        if credentials.contains_key(cred_key) {
            continue;
        }
        match provider_tokens.find_provider_token(user_id, provider).await {
            Ok(Some(row)) => {
                credentials.insert(cred_key.to_string(), row.access_token);
            }
            Ok(None) => {}
            Err(e) => {
                warn!(error = %e, user_id, provider, "Failed to query provider_token");
            }
        }
    }

    if credentials.contains_key(CLAUDE_CODE_OAUTH_TOKEN)
        || credentials.contains_key(ANTHROPIC_API_KEY)
    {
        return;
    }
    match provider_tokens.find_provider_token(user_id, "claude").await {
        Ok(Some(row)) => {
            credentials.insert(CLAUDE_CODE_OAUTH_TOKEN.to_string(), row.access_token);
        }
        Ok(None) => {}
        Err(e) => {
            warn!(error = %e, user_id, "Failed to query claude provider_token");
        }
    }
}
