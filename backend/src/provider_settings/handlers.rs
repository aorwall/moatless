//! Provider settings HTTP handlers (GitHub, Gitness).

use axum::extract::State;
use axum::Json;

use crate::extractors::AuthUser;
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
        (status = 401, description = "Not authenticated", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn get_github_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<GitHubConfigResponse>, ProviderSettingsError> {
    let token = state.provider_tokens.find_provider_token_full(&auth.user_id, "github")
        .await?;

    match token {
        Some(row) => {
            let login = row
                .metadata
                .as_ref()
                .and_then(|m| m.get("login"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string);

            Ok(Json(GitHubConfigResponse {
                connected: true,
                login,
                token_type: row.token_type,
            }))
        }
        None => Ok(Json(GitHubConfigResponse {
            connected: false,
            login: None,
            token_type: None,
        })),
    }
}

/// Save a GitHub Personal Access Token after validating it.
#[utoipa::path(
    put,
    path = "/api/v1/settings/github/pat",
    tag = "provider-settings",
    request_body = SavePatRequest,
    responses(
        (status = 200, description = "PAT saved", body = GitHubConfigResponse),
        (status = 400, description = "Invalid token", body = crate::sandbox::schemas::ErrorBody)
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

    let metadata = serde_json::json!({ "login": login });

    state.provider_tokens.upsert_token(
        &auth.user_id,
        "github",
        &req.token,
        None,
        Some("pat"),
        Some(&metadata),
    )
    .await?;

    Ok(Json(GitHubConfigResponse {
        connected: true,
        login: Some(login),
        token_type: Some("pat".to_string()),
    }))
}

/// Generate a GitHub OAuth authorization URL.
#[utoipa::path(
    get,
    path = "/api/v1/settings/github/oauth/url",
    tag = "provider-settings",
    responses(
        (status = 200, description = "OAuth URL", body = OAuthUrlResponse),
        (status = 400, description = "GitHub OAuth not configured", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn get_github_oauth_url(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> Result<Json<OAuthUrlResponse>, ProviderSettingsError> {
    let client_id = state
        .config
        .auth.github_client_id
        .as_deref()
        .ok_or_else(|| {
            ProviderSettingsError::BadRequest(
                "GitHub OAuth not configured (GITHUB_CLIENT_ID missing)".to_string(),
            )
        })?;

    let default_callback = format!("{}/settings/github/callback", state.config.auth.frontend_url);
    let callback_url = state
        .config
        .auth.github_callback_url
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
        (status = 400, description = "Exchange failed", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn exchange_github_token(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<ExchangeTokenRequest>,
) -> Result<Json<ExchangeTokenResponse>, ProviderSettingsError> {
    let client_id = state
        .config
        .auth.github_client_id
        .as_deref()
        .ok_or_else(|| {
            ProviderSettingsError::BadRequest("GitHub OAuth not configured".to_string())
        })?;

    let client_secret = state
        .config
        .auth.github_client_secret
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

    state.provider_tokens.upsert_token(
        &auth.user_id,
        "github",
        &access_token,
        None,
        Some(&token_type),
        Some(&metadata),
    )
    .await?;

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
        (status = 401, description = "Not authenticated", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn delete_github_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<GitHubConfigResponse>, ProviderSettingsError> {
    state.provider_tokens.delete_token(&auth.user_id, "github").await?;

    Ok(Json(GitHubConfigResponse {
        connected: false,
        login: None,
        token_type: None,
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
        (status = 401, description = "Not authenticated", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn get_gitness_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<GitnessConfigResponse>, ProviderSettingsError> {
    let token = state.provider_tokens.find_provider_token_full(&auth.user_id, "gitness")
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
        (status = 400, description = "Invalid token", body = crate::sandbox::schemas::ErrorBody)
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

    state.provider_tokens.upsert_token(
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
        (status = 401, description = "Not authenticated", body = crate::sandbox::schemas::ErrorBody)
    )
)]
pub async fn delete_gitness_config(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<GitnessConfigResponse>, ProviderSettingsError> {
    state.provider_tokens.delete_token(&auth.user_id, "gitness").await?;

    Ok(Json(GitnessConfigResponse {
        connected: false,
        login: None,
        base_url: None,
    }))
}
