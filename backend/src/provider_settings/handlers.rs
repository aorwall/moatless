//! Provider settings HTTP handlers (GitHub, Gitness, Codex).

use axum::extract::State;
use axum::Json;

use crate::auth::credentials::{
    read_github_oauth_stash, stash_github_oauth_token, GITHUB_OAUTH_STASH_KEY,
};
use crate::auth::provider_tokens::{AUTH_METHOD_SECRET_REF, SECRET_REF_TOKEN_PLACEHOLDER};
use crate::credentials::codex::oauth::{self as codex_oauth, PollStatus};
use crate::credentials::codex::{save_codex_auth_for_scope, CodexScope};
use crate::credentials::github::scope::GitHubScope;
use crate::credentials::WriteSource;
use crate::extractors::AuthUser;
use crate::sandbox::secrets::GITHUB_TOKEN;
use crate::secret::models::{SecretKind, SecretScopeType};
use crate::secret::schemas::PutSecretRequest;
use crate::state::AppState;

use super::error::ProviderSettingsError;
use super::github_client;
use super::gitness_client;
use super::schemas::*;

/// Get current GitHub configuration for the user.
#[utoipa::path(
    get,
    path = "/api/v1/settings/github/config",
    tag = "provider-settings",
    responses(
        (status = 200, description = "GitHub config", body = GitHubConfigResponse),
        (status = 401, description = "Not authenticated", body = crate::error::ErrorBody)
    )
)]
pub async fn get_github_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<GitHubConfigResponse>, ProviderSettingsError> {
    let token = state
        .provider_tokens
        .find_provider_token_full(&auth.user_id, "github")
        .await?;

    let is_github_oauth_user = auth.provider.as_deref() == Some("github");

    match token {
        Some(row) => {
            let login = row
                .metadata
                .as_ref()
                .and_then(|m| m.get("login"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string);

            let has_pat_override = if is_github_oauth_user {
                Some(row.token_type.as_deref() == Some("pat"))
            } else {
                None
            };

            Ok(Json(GitHubConfigResponse {
                connected: true,
                login,
                token_type: row.token_type,
                has_pat_override,
            }))
        }
        None => Ok(Json(GitHubConfigResponse {
            connected: false,
            login: None,
            token_type: None,
            has_pat_override: if is_github_oauth_user {
                Some(false)
            } else {
                None
            },
        })),
    }
}

// ============================================================================
// Org-wide (global) GitHub token — admin only
// ============================================================================

fn require_admin(auth: &AuthUser, action: &str) -> Result<(), ProviderSettingsError> {
    if auth.role.as_deref() == Some("admin") {
        return Ok(());
    }
    Err(ProviderSettingsError::Forbidden(format!(
        "Admin role required to {action} the organization GitHub token"
    )))
}

/// Read the org-wide GitHub token's status. Admin only.
#[utoipa::path(
    get,
    path = "/api/v1/admin/settings/github/config",
    tag = "provider-settings",
    responses(
        (status = 200, description = "Organization GitHub config", body = GitHubConfigResponse),
        (status = 401, description = "Not authenticated", body = crate::error::ErrorBody),
        (status = 403, description = "Admin only", body = crate::error::ErrorBody)
    )
)]
pub async fn admin_get_global_github_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<GitHubConfigResponse>, ProviderSettingsError> {
    require_admin(&auth, "read")?;

    let row = state
        .provider_tokens
        .find_github_row(&GitHubScope::Global)
        .await?;

    Ok(Json(match row {
        Some(row) => GitHubConfigResponse {
            connected: true,
            login: row
                .metadata
                .as_ref()
                .and_then(|m| m.get("login"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string),
            token_type: row.token_type,
            // A PAT override is a GitHub-OAuth-user concept; the org-wide
            // token is always a PAT, so there's nothing to override.
            has_pat_override: None,
        },
        None => GitHubConfigResponse {
            connected: false,
            login: None,
            token_type: None,
            has_pat_override: None,
        },
    }))
}

/// Set the org-wide GitHub token after validating it. Admin only.
///
/// Sits at the bottom of the `user > team > global` precedence chain: any user
/// with their own token shadows this one. It exists so a user who has never
/// connected GitHub can still read repositories.
#[utoipa::path(
    put,
    path = "/api/v1/admin/settings/github/pat",
    tag = "provider-settings",
    request_body = SavePatRequest,
    responses(
        (status = 200, description = "Organization PAT saved", body = GitHubConfigResponse),
        (status = 400, description = "Invalid token", body = crate::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = crate::error::ErrorBody),
        (status = 403, description = "Admin only", body = crate::error::ErrorBody)
    )
)]
pub async fn admin_save_global_github_pat(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<SavePatRequest>,
) -> Result<Json<GitHubConfigResponse>, ProviderSettingsError> {
    require_admin(&auth, "set")?;

    let login = github_client::validate_github_token(&req.token)
        .await
        .map_err(ProviderSettingsError::ProviderError)?;

    // Secret first, reference row second — a reference row pointing at a
    // missing secret is a hard failure on read. `put_secret` re-checks
    // admin-for-global via `ensure_scope_access`.
    state
        .secrets
        .put_secret(
            &auth.scope_context(),
            &auth.user_id,
            PutSecretRequest {
                scope_type: SecretScopeType::Global,
                scope_id: None,
                key: GITHUB_TOKEN.to_string(),
                kind: SecretKind::ProviderToken,
                provider: Some("github".to_string()),
                value: Some(req.token.clone()),
                external_ref: None,
            },
        )
        .await?;

    state
        .provider_tokens
        .upsert_github_row(
            &GitHubScope::Global,
            AUTH_METHOD_SECRET_REF,
            SECRET_REF_TOKEN_PLACEHOLDER,
            Some("pat"),
            Some(&serde_json::json!({ "login": login })),
        )
        .await?;

    Ok(Json(GitHubConfigResponse {
        connected: true,
        login: Some(login),
        token_type: Some("pat".to_string()),
        has_pat_override: None,
    }))
}

/// Remove the org-wide GitHub token. Admin only.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/settings/github/pat",
    tag = "provider-settings",
    responses(
        (status = 200, description = "Organization PAT removed", body = GitHubConfigResponse),
        (status = 401, description = "Not authenticated", body = crate::error::ErrorBody),
        (status = 403, description = "Admin only", body = crate::error::ErrorBody)
    )
)]
pub async fn admin_delete_global_github_pat(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<GitHubConfigResponse>, ProviderSettingsError> {
    require_admin(&auth, "remove")?;

    // Reference row first: while both exist the row resolves fine, but a row
    // outliving its secret is a hard error for every reader.
    state
        .provider_tokens
        .delete_github_row(&GitHubScope::Global)
        .await?;

    state
        .secrets
        .delete_scope_secret(
            &auth.scope_context(),
            &auth.user_id,
            SecretScopeType::Global,
            None,
            GITHUB_TOKEN,
            Some(SecretKind::ProviderToken),
        )
        .await?;

    Ok(Json(GitHubConfigResponse {
        connected: false,
        login: None,
        token_type: None,
        has_pat_override: None,
    }))
}

/// Save a GitHub Personal Access Token after validating it.
#[utoipa::path(
    put,
    path = "/api/v1/settings/github/pat",
    tag = "provider-settings",
    request_body = SavePatRequest,
    responses(
        (status = 200, description = "PAT saved", body = GitHubConfigResponse),
        (status = 400, description = "Invalid token", body = crate::error::ErrorBody)
    )
)]
pub async fn save_github_pat(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<SavePatRequest>,
) -> Result<Json<GitHubConfigResponse>, ProviderSettingsError> {
    // Validate the token by calling GitHub API
    let login = github_client::validate_github_token(&req.token)
        .await
        .map_err(ProviderSettingsError::ProviderError)?;

    let mut metadata = serde_json::json!({ "login": login });

    // For GitHub-OAuth users, preserve the original OAuth token so it can be restored later
    if auth.provider.as_deref() == Some("github") {
        if let Ok(Some(existing)) = state
            .provider_tokens
            .find_provider_token_full(&auth.user_id, "github")
            .await
        {
            if existing.token_type.as_deref() == Some("pat") {
                // Already a PAT override — the stash is already in place; only
                // the non-secret restore hints need carrying forward.
                for key in ["oauth_login", "oauth_auth_method"] {
                    if let Some(value) = existing
                        .metadata
                        .as_ref()
                        .and_then(|m| m.get(key))
                        .and_then(|v| v.as_str())
                    {
                        metadata[key] = serde_json::Value::String(value.to_string());
                    }
                }
            } else {
                stash_github_oauth_token(&state.secrets, &auth.user_id, &existing.access_token)
                    .await?;
                if let Some(oauth_login) = existing
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("login"))
                    .and_then(|v| v.as_str())
                {
                    metadata["oauth_login"] = serde_json::Value::String(oauth_login.to_string());
                }
                // Remember how the connection authenticated so removing the
                // override restores it — a `github_app` user must get their
                // refreshable App branch back, not a plain `oauth` row.
                if let Some(auth_method) = existing.auth_method.as_deref() {
                    metadata["oauth_auth_method"] =
                        serde_json::Value::String(auth_method.to_string());
                }
            }
        }
    }

    // The PAT value lives in the encrypted `secrets` table; the
    // `provider_tokens` row is kept as a reference so row-presence consumers
    // (config `connected`, `auth/me` providers, PAT-override detection) keep
    // working. Write the secret first — a reference row pointing at a missing
    // secret is a hard failure on read.
    state
        .secrets
        .put_secret(
            &auth.scope_context(),
            &auth.user_id,
            PutSecretRequest {
                scope_type: SecretScopeType::User,
                scope_id: Some(auth.user_id.clone()),
                key: GITHUB_TOKEN.to_string(),
                kind: SecretKind::ProviderToken,
                provider: Some("github".to_string()),
                value: Some(req.token.clone()),
                external_ref: None,
            },
        )
        .await?;

    state
        .provider_tokens
        .upsert_token_with_auth_method(
            &auth.user_id,
            "github",
            AUTH_METHOD_SECRET_REF,
            SECRET_REF_TOKEN_PLACEHOLDER,
            None,
            Some("pat"),
            Some(&metadata),
        )
        .await?;

    let is_github_oauth_user = auth.provider.as_deref() == Some("github");

    Ok(Json(GitHubConfigResponse {
        connected: true,
        login: Some(login),
        token_type: Some("pat".to_string()),
        has_pat_override: if is_github_oauth_user {
            Some(true)
        } else {
            None
        },
    }))
}

/// Generate a GitHub OAuth authorization URL.
#[utoipa::path(
    get,
    path = "/api/v1/settings/github/oauth/url",
    tag = "provider-settings",
    responses(
        (status = 200, description = "OAuth URL", body = OAuthUrlResponse),
        (status = 400, description = "GitHub OAuth not configured", body = crate::error::ErrorBody)
    )
)]
pub async fn get_github_oauth_url(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<OAuthUrlResponse>, ProviderSettingsError> {
    let client_id = state
        .config
        .auth
        .github_client_id
        .as_deref()
        .ok_or_else(|| {
            ProviderSettingsError::BadRequest(
                "GitHub OAuth not configured (GITHUB_CLIENT_ID missing)".to_string(),
            )
        })?;

    let default_callback = format!(
        "{}/settings/github/callback",
        state.config.auth.frontend_url
    );
    let callback_url = state
        .config
        .auth
        .github_callback_url
        .as_deref()
        .unwrap_or(&default_callback);

    let scope = &state.config.auth.github_oauth_scope;
    let state_param = uuid::Uuid::new_v4().to_string();

    let url = format!(
        "https://github.com/login/oauth/authorize?client_id={}&redirect_uri={}&scope={}&state={}",
        urlencoding::encode(client_id),
        urlencoding::encode(callback_url),
        urlencoding::encode(scope),
        urlencoding::encode(&state_param),
    );

    Ok(Json(OAuthUrlResponse { url }))
}

/// Exchange a GitHub OAuth authorization code for an access token.
#[utoipa::path(
    post,
    path = "/api/v1/settings/github/oauth/token",
    tag = "provider-settings",
    request_body = ExchangeTokenRequest,
    responses(
        (status = 200, description = "Token exchanged", body = ExchangeTokenResponse),
        (status = 400, description = "Exchange failed", body = crate::error::ErrorBody)
    )
)]
pub async fn exchange_github_token(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<ExchangeTokenRequest>,
) -> Result<Json<ExchangeTokenResponse>, ProviderSettingsError> {
    let client_id = state
        .config
        .auth
        .github_client_id
        .as_deref()
        .ok_or_else(|| {
            ProviderSettingsError::BadRequest("GitHub OAuth not configured".to_string())
        })?;

    let client_secret = state
        .config
        .auth
        .github_client_secret
        .as_deref()
        .ok_or_else(|| {
            ProviderSettingsError::BadRequest("GitHub OAuth not configured".to_string())
        })?;

    let (access_token, token_type) =
        github_client::exchange_github_oauth_code(client_id, client_secret, &req.code)
            .await
            .map_err(ProviderSettingsError::ProviderError)?;

    // Validate the token to get the login
    let login = github_client::validate_github_token(&access_token)
        .await
        .map_err(ProviderSettingsError::ProviderError)?;

    let metadata = serde_json::json!({ "login": login });

    // Set `auth_method` explicitly: this row now carries a real OAuth token, so
    // a previous `secret_ref` marker must not survive and send the resolver
    // looking for a stale secret.
    state
        .provider_tokens
        .upsert_token_with_auth_method(
            &auth.user_id,
            "github",
            "oauth",
            &access_token,
            None,
            Some(&token_type),
            Some(&metadata),
        )
        .await?;

    for (key, kind) in [
        (GITHUB_TOKEN, SecretKind::ProviderToken),
        (GITHUB_OAUTH_STASH_KEY, SecretKind::Internal),
    ] {
        if let Err(e) = state
            .secrets
            .delete_user_secret(&auth.user_id, key, Some(kind))
            .await
        {
            tracing::warn!(error = %e, user_id = %auth.user_id, key, "Failed to clean up secret on GitHub OAuth exchange");
        }
    }

    Ok(Json(ExchangeTokenResponse {
        connected: true,
        login: Some(login),
    }))
}

/// Disconnect GitHub by deleting the stored token.
#[utoipa::path(
    delete,
    path = "/api/v1/settings/github/config",
    tag = "provider-settings",
    responses(
        (status = 200, description = "GitHub disconnected", body = GitHubConfigResponse),
        (status = 401, description = "Not authenticated", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_github_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<GitHubConfigResponse>, ProviderSettingsError> {
    state
        .provider_tokens
        .delete_token(&auth.user_id, "github")
        .await?;

    for (key, kind) in [
        (GITHUB_TOKEN, SecretKind::ProviderToken),
        (GITHUB_OAUTH_STASH_KEY, SecretKind::Internal),
    ] {
        if let Err(e) = state
            .secrets
            .delete_user_secret(&auth.user_id, key, Some(kind))
            .await
        {
            tracing::warn!(error = %e, user_id = %auth.user_id, key, "Failed to clean up secret on GitHub disconnect");
        }
    }

    Ok(Json(GitHubConfigResponse {
        connected: false,
        login: None,
        token_type: None,
        has_pat_override: None,
    }))
}

/// Remove a PAT override for a GitHub-OAuth user, restoring the original OAuth token.
#[utoipa::path(
    delete,
    path = "/api/v1/settings/github/pat",
    tag = "provider-settings",
    responses(
        (status = 200, description = "PAT override removed", body = GitHubConfigResponse),
        (status = 400, description = "No PAT override to remove", body = crate::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_github_pat_override(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<GitHubConfigResponse>, ProviderSettingsError> {
    if auth.provider.as_deref() != Some("github") {
        return Err(ProviderSettingsError::BadRequest(
            "PAT override removal is only available for GitHub-OAuth users".to_string(),
        ));
    }

    let token = state
        .provider_tokens
        .find_provider_token_full(&auth.user_id, "github")
        .await?
        .ok_or_else(|| ProviderSettingsError::NotFound("No GitHub token found".to_string()))?;

    if token.token_type.as_deref() != Some("pat") {
        return Err(ProviderSettingsError::BadRequest(
            "No PAT override to remove".to_string(),
        ));
    }

    let oauth_token = read_github_oauth_stash(&state.secrets, &auth.user_id)
        .await?
        .ok_or_else(|| {
            ProviderSettingsError::BadRequest(
                "Cannot restore OAuth token — original token not preserved".to_string(),
            )
        })?;

    let oauth_login = token
        .metadata
        .as_ref()
        .and_then(|m| m.get("oauth_login"))
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    // Restore the branch the connection used before the override; plain OAuth
    // is the fallback for rows stashed without one.
    let restored_auth_method = token
        .metadata
        .as_ref()
        .and_then(|m| m.get("oauth_auth_method"))
        .and_then(|v| v.as_str())
        .unwrap_or("oauth")
        .to_string();

    let login = oauth_login.unwrap_or_else(|| auth.login.clone());
    let metadata = serde_json::json!({ "login": login });

    // Reset `auth_method` off `secret_ref` — the row carries a usable OAuth
    // token again, so the resolver must stop looking for a secret.
    state
        .provider_tokens
        .upsert_token_with_auth_method(
            &auth.user_id,
            "github",
            &restored_auth_method,
            &oauth_token,
            None,
            Some("bearer"),
            Some(&metadata),
        )
        .await?;

    // The override is gone; drop both the PAT and the stash rather than
    // leaving decryptable credentials behind.
    for (key, kind) in [
        (GITHUB_TOKEN, SecretKind::ProviderToken),
        (GITHUB_OAUTH_STASH_KEY, SecretKind::Internal),
    ] {
        if let Err(e) = state
            .secrets
            .delete_user_secret(&auth.user_id, key, Some(kind))
            .await
        {
            tracing::warn!(error = %e, user_id = %auth.user_id, key, "Failed to clean up secret after PAT removal");
        }
    }

    Ok(Json(GitHubConfigResponse {
        connected: true,
        login: Some(login),
        token_type: Some("bearer".to_string()),
        has_pat_override: Some(false),
    }))
}

// =========================================================================
// Gitness handlers
// =========================================================================

/// Get current Gitness configuration for the user.
#[utoipa::path(
    get,
    path = "/api/v1/settings/gitness/config",
    tag = "provider-settings",
    responses(
        (status = 200, description = "Gitness config", body = GitnessConfigResponse),
        (status = 401, description = "Not authenticated", body = crate::error::ErrorBody)
    )
)]
pub async fn get_gitness_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<GitnessConfigResponse>, ProviderSettingsError> {
    let token = state
        .provider_tokens
        .find_provider_token_full(&auth.user_id, "gitness")
        .await?;

    match token {
        Some(row) => {
            let login = row
                .metadata
                .as_ref()
                .and_then(|m| m.get("login"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string);

            let base_url = row
                .metadata
                .as_ref()
                .and_then(|m| m.get("base_url"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string);

            Ok(Json(GitnessConfigResponse {
                connected: true,
                login,
                base_url,
            }))
        }
        None => Ok(Json(GitnessConfigResponse {
            connected: false,
            login: None,
            base_url: None,
        })),
    }
}

/// Save a Gitness Personal Access Token after validating it.
#[utoipa::path(
    put,
    path = "/api/v1/settings/gitness/pat",
    tag = "provider-settings",
    request_body = SaveGitnessPATRequest,
    responses(
        (status = 200, description = "PAT saved", body = GitnessConfigResponse),
        (status = 400, description = "Invalid token", body = crate::error::ErrorBody)
    )
)]
pub async fn save_gitness_pat(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<SaveGitnessPATRequest>,
) -> Result<Json<GitnessConfigResponse>, ProviderSettingsError> {
    // Validate the token by calling Gitness API
    let login = gitness_client::validate_gitness_token(&req.base_url, &req.token)
        .await
        .map_err(ProviderSettingsError::ProviderError)?;

    let metadata = serde_json::json!({
        "login": login,
        "base_url": req.base_url,
    });

    state
        .provider_tokens
        .upsert_token(
            &auth.user_id,
            "gitness",
            &req.token,
            None,
            Some("pat"),
            Some(&metadata),
        )
        .await?;

    Ok(Json(GitnessConfigResponse {
        connected: true,
        login: Some(login),
        base_url: Some(req.base_url),
    }))
}

/// Disconnect Gitness by deleting the stored token.
#[utoipa::path(
    delete,
    path = "/api/v1/settings/gitness/config",
    tag = "provider-settings",
    responses(
        (status = 200, description = "Gitness disconnected", body = GitnessConfigResponse),
        (status = 401, description = "Not authenticated", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_gitness_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<GitnessConfigResponse>, ProviderSettingsError> {
    state
        .provider_tokens
        .delete_token(&auth.user_id, "gitness")
        .await?;

    Ok(Json(GitnessConfigResponse {
        connected: false,
        login: None,
        base_url: None,
    }))
}

// =========================================================================
// Codex handlers
// =========================================================================

/// Get current Codex configuration for the user.
#[utoipa::path(
    get,
    path = "/api/v1/settings/codex/config",
    tag = "provider-settings",
    responses(
        (status = 200, description = "Codex config", body = CodexConfigResponse),
        (status = 401, description = "Not authenticated", body = crate::error::ErrorBody)
    )
)]
pub async fn get_codex_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<CodexConfigResponse>, ProviderSettingsError> {
    let token = state
        .provider_tokens
        .find_provider_token_full(&auth.user_id, "codex")
        .await?;

    match token {
        Some(row) => {
            let metadata = row.metadata.as_ref();
            let auth_method = metadata
                .and_then(|m| m.get("auth_method"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string);
            let email = metadata
                .and_then(|m| m.get("email"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string);
            let plan_type = metadata
                .and_then(|m| m.get("plan_type"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string);
            // Written by the refresher on permanent refresh failure; cleared
            // on the next successful save/refresh.
            let needs_reconnect = metadata.is_some_and(|m| m.get("refresh_failure").is_some());

            Ok(Json(CodexConfigResponse {
                connected: true,
                token_type: row.token_type,
                auth_method,
                email,
                plan_type,
                needs_reconnect: Some(needs_reconnect),
            }))
        }
        None => Ok(Json(CodexConfigResponse {
            connected: false,
            token_type: None,
            auth_method: None,
            email: None,
            plan_type: None,
            needs_reconnect: None,
        })),
    }
}

/// Save Codex auth.json contents for sandbox startup.
#[utoipa::path(
    put,
    path = "/api/v1/settings/codex/config",
    tag = "provider-settings",
    request_body = SaveCodexAuthRequest,
    responses(
        (status = 200, description = "Codex auth saved", body = CodexConfigResponse),
        (status = 400, description = "Invalid auth JSON", body = crate::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = crate::error::ErrorBody)
    )
)]
pub async fn save_codex_auth(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<SaveCodexAuthRequest>,
) -> Result<Json<CodexConfigResponse>, ProviderSettingsError> {
    let resp = save_codex_auth_for_scope(&state, CodexScope::user(&auth.user_id), req).await?;
    Ok(Json(resp))
}

/// Save the instance-wide (global) Codex blob. Admin only.
///
/// Sits at the bottom of the precedence chain: any user with their own row
/// or a member of a team with a team row will shadow this. Useful for
/// seeding bots (e.g. `bot:reviewer`) without per-bot upserts.
#[utoipa::path(
    post,
    path = "/api/v1/admin/settings/codex/config",
    tag = "provider-settings",
    request_body = SaveCodexAuthRequest,
    responses(
        (status = 200, description = "Global codex auth saved", body = CodexConfigResponse),
        (status = 400, description = "Invalid auth JSON", body = crate::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = crate::error::ErrorBody),
        (status = 403, description = "Admin only", body = crate::error::ErrorBody)
    )
)]
pub async fn admin_save_global_codex_auth(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<SaveCodexAuthRequest>,
) -> Result<Json<CodexConfigResponse>, ProviderSettingsError> {
    if auth.role.as_deref() != Some("admin") {
        return Err(ProviderSettingsError::Forbidden(
            "Admin role required to set the global Codex blob".to_string(),
        ));
    }
    let resp = save_codex_auth_for_scope(&state, CodexScope::Global, req).await?;
    Ok(Json(resp))
}

/// Delete the instance-wide (global) Codex blob. Admin only.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/settings/codex/config",
    tag = "provider-settings",
    responses(
        (status = 200, description = "Global codex disconnected", body = CodexConfigResponse),
        (status = 401, description = "Not authenticated", body = crate::error::ErrorBody),
        (status = 403, description = "Admin only", body = crate::error::ErrorBody)
    )
)]
pub async fn admin_delete_global_codex_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<CodexConfigResponse>, ProviderSettingsError> {
    if auth.role.as_deref() != Some("admin") {
        return Err(ProviderSettingsError::Forbidden(
            "Admin role required to delete the global Codex blob".to_string(),
        ));
    }
    state
        .provider_tokens
        .delete_codex_blob(&CodexScope::Global)
        .await?;

    Ok(Json(CodexConfigResponse {
        connected: false,
        token_type: None,
        auth_method: None,
        email: None,
        plan_type: None,
        needs_reconnect: None,
    }))
}

/// Disconnect Codex by deleting the stored auth blob.
#[utoipa::path(
    delete,
    path = "/api/v1/settings/codex/config",
    tag = "provider-settings",
    responses(
        (status = 200, description = "Codex disconnected", body = CodexConfigResponse),
        (status = 401, description = "Not authenticated", body = crate::error::ErrorBody)
    )
)]
pub async fn delete_codex_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<CodexConfigResponse>, ProviderSettingsError> {
    state
        .provider_tokens
        .delete_codex_blob(&CodexScope::user(&auth.user_id))
        .await?;

    Ok(Json(CodexConfigResponse {
        connected: false,
        token_type: None,
        auth_method: None,
        email: None,
        plan_type: None,
        needs_reconnect: None,
    }))
}

// =========================================================================
// Codex device-code OAuth handlers
// =========================================================================

/// Begin a Codex device-code OAuth flow. The frontend shows the returned
/// `userCode` to the user and points them at `verificationUrl` to approve.
#[utoipa::path(
    post,
    path = "/api/v1/settings/codex/login/start",
    tag = "provider-settings",
    responses(
        (status = 200, description = "Device code issued", body = CodexDeviceCodeStartResponse),
        (status = 401, description = "Not authenticated", body = crate::error::ErrorBody),
        (status = 502, description = "Codex OAuth provider error", body = crate::error::ErrorBody)
    )
)]
pub async fn start_codex_device_login(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<CodexDeviceCodeStartResponse>, ProviderSettingsError> {
    let client = reqwest::Client::new();
    let started = codex_oauth::request_device_code(
        &client,
        &state.config.codex_oauth.issuer,
        &state.config.codex_oauth.client_id,
    )
    .await
    .map_err(|e| ProviderSettingsError::ProviderError(e.to_string()))?;

    Ok(Json(CodexDeviceCodeStartResponse {
        device_auth_id: started.device_auth_id,
        user_code: started.user_code,
        verification_url: started.verification_url,
        interval_secs: started.interval_secs,
        expires_in_secs: started.expires_in_secs,
    }))
}

/// Poll one tick of the Codex device-code flow. The frontend drives the
/// cadence (~5s typical). On success, we persist the assembled auth.json blob
/// to `provider_tokens` and return the connected metadata.
#[utoipa::path(
    post,
    path = "/api/v1/settings/codex/login/poll",
    tag = "provider-settings",
    request_body = CodexDeviceCodePollRequest,
    responses(
        (status = 200, description = "Poll status", body = CodexDeviceCodePollResponse),
        (status = 401, description = "Not authenticated", body = crate::error::ErrorBody),
        (status = 502, description = "Codex OAuth provider error", body = crate::error::ErrorBody)
    )
)]
pub async fn poll_codex_device_login(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CodexDeviceCodePollRequest>,
) -> Result<Json<CodexDeviceCodePollResponse>, ProviderSettingsError> {
    let client = reqwest::Client::new();
    let outcome = codex_oauth::poll_once(
        &client,
        &state.config.codex_oauth.issuer,
        &state.config.codex_oauth.client_id,
        &req.device_auth_id,
        &req.user_code,
    )
    .await
    .map_err(|e| ProviderSettingsError::ProviderError(e.to_string()))?;

    match outcome {
        PollStatus::Pending => Ok(Json(CodexDeviceCodePollResponse::Pending)),
        PollStatus::Expired => Ok(Json(CodexDeviceCodePollResponse::Expired)),
        PollStatus::Success {
            auth_json,
            id_token_claims,
        } => {
            let metadata = serde_json::json!({
                "auth_method": "device_code",
                "has_tokens": true,
                "email": id_token_claims.email,
                "plan_type": id_token_claims.chatgpt_plan_type,
                "account_id": id_token_claims.chatgpt_account_id,
            });

            let serialized = auth_json.to_string();
            state
                .provider_tokens
                .upsert_token(
                    &auth.user_id,
                    "codex",
                    &serialized,
                    None,
                    Some("auth_json"),
                    Some(&metadata),
                )
                .await?;

            // Broadcast the freshly-minted Codex auth.json to any running
            // sandboxes the user owns so they pick up the new tokens without
            // a restart.
            if let Err(e) = state
                .credentials_svc
                .store(
                    &auth.user_id,
                    "CODEX_AUTH_JSON",
                    &serialized,
                    WriteSource::UserUiSave,
                )
                .await
            {
                tracing::warn!(
                    error = %e,
                    "Failed to broadcast Codex device-code login to running sandboxes"
                );
            }

            Ok(Json(CodexDeviceCodePollResponse::Success {
                connected: true,
                email: id_token_claims.email,
                plan_type: id_token_claims.chatgpt_plan_type,
            }))
        }
    }
}
