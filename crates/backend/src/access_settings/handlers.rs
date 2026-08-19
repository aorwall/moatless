//! Thin HTTP handlers for access administration.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};

use crate::access_settings::error::AccessSettingsError;
use crate::access_settings::schemas::*;
use crate::agent_harness_access::{
    AgentHarnessAccess, AgentHarnessCredentialStatus, DeviceLoginPoll,
};
use crate::app_administration::{
    AdapterAppSave, AppAdministration, GitHubAppRegistration, GitHubAppRegistrationInput,
    GitHubAppView,
};
use moatless_types::agent::CodexScope;
use crate::extractors::AuthUser;
use moatless_identity::git_host_access::{
    GitHostAccess, GitHostPrincipal, GitHubProviderTokenStatus, GitnessProviderTokenStatus,
};

fn require_admin(auth: &AuthUser, action: &str) -> Result<(), AccessSettingsError> {
    if auth.role.as_deref() == Some("admin") {
        return Ok(());
    }
    Err(AccessSettingsError::Forbidden(format!(
        "Admin role required to {action} provider settings"
    )))
}

fn principal(auth: &AuthUser) -> GitHostPrincipal {
    GitHostPrincipal {
        user_id: auth.user_id.to_string(),
        login: auth.login.clone(),
        is_github_app_user: auth.provider.as_deref() == Some("github_app"),
        scope_context: auth.scope_context(),
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/settings/github/config",
    tag = "git-host-access",
    responses(
        (status = 200, description = "GitHub config", body = GitHubProviderTokenStatusResponse),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_github_config(
    State(access): State<Arc<GitHostAccess>>,
    auth: AuthUser,
) -> Result<Json<GitHubProviderTokenStatusResponse>, AccessSettingsError> {
    Ok(Json(github_status(
        access.github_status(&principal(&auth)).await?,
    )))
}

#[utoipa::path(
    put,
    path = "/api/v1/settings/github/pat",
    tag = "git-host-access",
    request_body = SaveGitHubProviderTokenRequest,
    responses(
        (status = 200, description = "PAT saved", body = GitHubProviderTokenStatusResponse),
        (status = 400, description = "Invalid token", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn save_github_pat(
    State(access): State<Arc<GitHostAccess>>,
    auth: AuthUser,
    Json(request): Json<SaveGitHubProviderTokenRequest>,
) -> Result<Json<GitHubProviderTokenStatusResponse>, AccessSettingsError> {
    Ok(Json(github_status(
        access
            .save_github_provider_token(&principal(&auth), request.token)
            .await?,
    )))
}

#[utoipa::path(
    delete,
    path = "/api/v1/settings/github/config",
    tag = "git-host-access",
    responses(
        (status = 200, description = "GitHub disconnected", body = GitHubProviderTokenStatusResponse),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn delete_github_config(
    State(access): State<Arc<GitHostAccess>>,
    auth: AuthUser,
) -> Result<Json<GitHubProviderTokenStatusResponse>, AccessSettingsError> {
    Ok(Json(github_status(
        access.disconnect_github(&principal(&auth)).await?,
    )))
}

#[utoipa::path(
    delete,
    path = "/api/v1/settings/github/pat",
    tag = "git-host-access",
    responses(
        (status = 200, description = "PAT override removed", body = GitHubProviderTokenStatusResponse),
        (status = 400, description = "No PAT override to remove", body = moatless_api_schemas::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn delete_github_pat_override(
    State(access): State<Arc<GitHostAccess>>,
    auth: AuthUser,
) -> Result<Json<GitHubProviderTokenStatusResponse>, AccessSettingsError> {
    Ok(Json(github_status(
        access.restore_github_app_token(&principal(&auth)).await?,
    )))
}

#[utoipa::path(
    get,
    path = "/api/v1/settings/gitness/config",
    tag = "git-host-access",
    responses(
        (status = 200, description = "Gitness config", body = GitnessProviderTokenStatusResponse),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_gitness_config(
    State(access): State<Arc<GitHostAccess>>,
    auth: AuthUser,
) -> Result<Json<GitnessProviderTokenStatusResponse>, AccessSettingsError> {
    Ok(Json(gitness_status(
        access.gitness_status(&auth.user_id.to_string()).await?,
    )))
}

#[utoipa::path(
    put,
    path = "/api/v1/settings/gitness/pat",
    tag = "git-host-access",
    request_body = SaveGitnessProviderTokenRequest,
    responses(
        (status = 200, description = "PAT saved", body = GitnessProviderTokenStatusResponse),
        (status = 400, description = "Invalid token", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn save_gitness_pat(
    State(access): State<Arc<GitHostAccess>>,
    auth: AuthUser,
    Json(request): Json<SaveGitnessProviderTokenRequest>,
) -> Result<Json<GitnessProviderTokenStatusResponse>, AccessSettingsError> {
    Ok(Json(gitness_status(
        access
            .save_gitness_provider_token(&auth.user_id.to_string(), request.base_url, request.token)
            .await?,
    )))
}

#[utoipa::path(
    delete,
    path = "/api/v1/settings/gitness/config",
    tag = "git-host-access",
    responses(
        (status = 200, description = "Gitness disconnected", body = GitnessProviderTokenStatusResponse),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn delete_gitness_config(
    State(access): State<Arc<GitHostAccess>>,
    auth: AuthUser,
) -> Result<Json<GitnessProviderTokenStatusResponse>, AccessSettingsError> {
    Ok(Json(gitness_status(
        access.disconnect_gitness(&auth.user_id.to_string()).await?,
    )))
}

#[utoipa::path(
    get,
    path = "/api/v1/settings/codex/config",
    tag = "agent-harness-access",
    responses(
        (status = 200, description = "Codex config", body = CodexAgentHarnessCredentialStatusResponse),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_codex_config(
    State(access): State<Arc<AgentHarnessAccess>>,
    auth: AuthUser,
) -> Result<Json<CodexAgentHarnessCredentialStatusResponse>, AccessSettingsError> {
    Ok(Json(agent_harness_status(
        access
            .status(&CodexScope::user(auth.user_id.to_string()))
            .await?,
    )))
}

#[utoipa::path(
    put,
    path = "/api/v1/settings/codex/config",
    tag = "agent-harness-access",
    request_body = SaveCodexAgentHarnessCredentialRequest,
    responses(
        (status = 200, description = "Codex auth saved", body = CodexAgentHarnessCredentialStatusResponse),
        (status = 400, description = "Invalid auth JSON", body = moatless_api_schemas::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn save_codex_auth(
    State(access): State<Arc<AgentHarnessAccess>>,
    auth: AuthUser,
    Json(request): Json<SaveCodexAgentHarnessCredentialRequest>,
) -> Result<Json<CodexAgentHarnessCredentialStatusResponse>, AccessSettingsError> {
    Ok(Json(agent_harness_status(
        access
            .save(
                CodexScope::user(auth.user_id.to_string()),
                request.auth_json,
            )
            .await?,
    )))
}

#[utoipa::path(
    delete,
    path = "/api/v1/settings/codex/config",
    tag = "agent-harness-access",
    responses(
        (status = 200, description = "Codex disconnected", body = CodexAgentHarnessCredentialStatusResponse),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn delete_codex_config(
    State(access): State<Arc<AgentHarnessAccess>>,
    auth: AuthUser,
) -> Result<Json<CodexAgentHarnessCredentialStatusResponse>, AccessSettingsError> {
    Ok(Json(agent_harness_status(
        access
            .delete(&CodexScope::user(auth.user_id.to_string()))
            .await?,
    )))
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/settings/codex/config",
    tag = "agent-harness-access",
    request_body = SaveCodexAgentHarnessCredentialRequest,
    responses(
        (status = 200, description = "Global codex auth saved", body = CodexAgentHarnessCredentialStatusResponse),
        (status = 400, description = "Invalid auth JSON", body = moatless_api_schemas::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn admin_save_global_codex_auth(
    State(access): State<Arc<AgentHarnessAccess>>,
    auth: AuthUser,
    Json(request): Json<SaveCodexAgentHarnessCredentialRequest>,
) -> Result<Json<CodexAgentHarnessCredentialStatusResponse>, AccessSettingsError> {
    if auth.role.as_deref() != Some("admin") {
        return Err(AccessSettingsError::Forbidden(
            "Admin role required to set the global Codex blob".to_string(),
        ));
    }
    Ok(Json(agent_harness_status(
        access.save(CodexScope::Global, request.auth_json).await?,
    )))
}

#[utoipa::path(
    delete,
    path = "/api/v1/admin/settings/codex/config",
    tag = "agent-harness-access",
    responses(
        (status = 200, description = "Global codex disconnected", body = CodexAgentHarnessCredentialStatusResponse),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn admin_delete_global_codex_config(
    State(access): State<Arc<AgentHarnessAccess>>,
    auth: AuthUser,
) -> Result<Json<CodexAgentHarnessCredentialStatusResponse>, AccessSettingsError> {
    if auth.role.as_deref() != Some("admin") {
        return Err(AccessSettingsError::Forbidden(
            "Admin role required to delete the global Codex blob".to_string(),
        ));
    }
    Ok(Json(agent_harness_status(
        access.delete(&CodexScope::Global).await?,
    )))
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/users/{user_id}/provider-tokens/codex",
    tag = "agent-harness-access",
    params(("user_id" = String, Path, description = "Target user id")),
    request_body = SaveCodexAgentHarnessCredentialRequest,
    responses(
        (status = 200, description = "Codex provider_token seeded", body = CodexAgentHarnessCredentialStatusResponse),
        (status = 400, description = "Invalid Codex auth JSON", body = moatless_api_schemas::error::ErrorBody),
        (status = 401, description = "Unauthorized", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_api_schemas::error::ErrorBody),
        (status = 500, description = "Internal error", body = moatless_api_schemas::error::ErrorBody),
    )
)]
pub async fn admin_set_codex_provider_token_handler(
    State(access): State<Arc<AgentHarnessAccess>>,
    auth: AuthUser,
    Path(target_user_id): Path<String>,
    Json(request): Json<SaveCodexAgentHarnessCredentialRequest>,
) -> Result<Json<CodexAgentHarnessCredentialStatusResponse>, AccessSettingsError> {
    if auth.role.as_deref() != Some("admin") {
        return Err(AccessSettingsError::AuthForbidden);
    }
    Ok(Json(agent_harness_status(
        access
            .save_for_user(&target_user_id, request.auth_json)
            .await?,
    )))
}

#[utoipa::path(
    post,
    path = "/api/v1/settings/codex/login/start",
    tag = "agent-harness-access",
    responses(
        (status = 200, description = "Device code issued", body = CodexDeviceCodeStartResponse),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody),
        (status = 502, description = "Codex OAuth provider error", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn start_codex_device_login(
    State(access): State<Arc<AgentHarnessAccess>>,
    _auth: AuthUser,
) -> Result<Json<CodexDeviceCodeStartResponse>, AccessSettingsError> {
    let started = access.start_device_login().await?;
    Ok(Json(CodexDeviceCodeStartResponse {
        device_auth_id: started.device_auth_id,
        user_code: started.user_code,
        verification_url: started.verification_url,
        interval_secs: started.interval_secs,
        expires_in_secs: started.expires_in_secs,
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/settings/codex/login/poll",
    tag = "agent-harness-access",
    request_body = CodexDeviceCodePollRequest,
    responses(
        (status = 200, description = "Poll status", body = CodexDeviceCodePollResponse),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody),
        (status = 502, description = "Codex OAuth provider error", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn poll_codex_device_login(
    State(access): State<Arc<AgentHarnessAccess>>,
    auth: AuthUser,
    Json(request): Json<CodexDeviceCodePollRequest>,
) -> Result<Json<CodexDeviceCodePollResponse>, AccessSettingsError> {
    let response = match access
        .poll_device_login(
            &auth.user_id.to_string(),
            &request.device_auth_id,
            &request.user_code,
        )
        .await?
    {
        DeviceLoginPoll::Pending => CodexDeviceCodePollResponse::Pending,
        DeviceLoginPoll::Expired => CodexDeviceCodePollResponse::Expired,
        DeviceLoginPoll::Success {
            connected,
            email,
            plan_type,
        } => CodexDeviceCodePollResponse::Success {
            connected,
            email,
            plan_type,
        },
    };
    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/settings/adapter-apps",
    tag = "app-administration",
    responses(
        (status = 200, description = "Configured Adapter apps", body = AdapterAppsResponse),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn admin_list_adapter_apps(
    State(administration): State<Arc<AppAdministration>>,
    auth: AuthUser,
) -> Result<Json<AdapterAppsResponse>, AccessSettingsError> {
    require_admin(&auth, "read")?;
    Ok(Json(AdapterAppsResponse {
        apps: administration
            .list_adapter_apps()
            .await?
            .into_iter()
            .map(adapter_app_summary)
            .collect(),
    }))
}

#[utoipa::path(
    put,
    path = "/api/v1/admin/settings/adapter-apps/{adapter_kind}/{app_key}",
    tag = "app-administration",
    params(
        ("adapter_kind" = String, Path, description = "Adapter kind"),
        ("app_key" = String, Path, description = "Adapter app key")
    ),
    request_body = SaveAdapterAppRequest,
    responses(
        (status = 200, description = "Adapter app saved", body = AdapterAppSummary),
        (status = 400, description = "Invalid Adapter app config", body = moatless_api_schemas::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn admin_save_adapter_app(
    State(administration): State<Arc<AppAdministration>>,
    auth: AuthUser,
    Path((adapter_kind, app_key)): Path<(String, String)>,
    Json(request): Json<SaveAdapterAppRequest>,
) -> Result<Json<AdapterAppSummary>, AccessSettingsError> {
    require_admin(&auth, "save")?;
    Ok(Json(adapter_app_summary(
        administration
            .save_adapter_app(
                &adapter_kind,
                &app_key,
                adapter_app_save(request),
                &auth.user_id.to_string(),
            )
            .await?,
    )))
}

#[utoipa::path(
    delete,
    path = "/api/v1/admin/settings/adapter-apps/{adapter_kind}/{app_key}",
    tag = "app-administration",
    params(
        ("adapter_kind" = String, Path, description = "Adapter kind"),
        ("app_key" = String, Path, description = "Adapter app key")
    ),
    responses(
        (status = 204, description = "Adapter app deleted"),
        (status = 400, description = "Invalid Adapter app", body = moatless_api_schemas::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn admin_delete_adapter_app(
    State(administration): State<Arc<AppAdministration>>,
    auth: AuthUser,
    Path((adapter_kind, app_key)): Path<(String, String)>,
) -> Result<axum::http::StatusCode, AccessSettingsError> {
    require_admin(&auth, "delete")?;
    administration
        .delete_adapter_app(&adapter_kind, &app_key)
        .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/settings/github/apps",
    tag = "app-administration",
    responses(
        (status = 200, description = "Configured GitHub Apps and installations", body = GitHubAppsResponse),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn admin_list_github_apps(
    State(administration): State<Arc<AppAdministration>>,
    auth: AuthUser,
) -> Result<Json<GitHubAppsResponse>, AccessSettingsError> {
    require_admin(&auth, "read")?;
    Ok(Json(GitHubAppsResponse {
        apps: administration
            .list_github_apps()
            .await?
            .into_iter()
            .map(github_app_option)
            .collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/settings/github/apps",
    tag = "app-administration",
    request_body = RegisterGitHubAppRequest,
    responses(
        (status = 201, description = "App proved against GitHub and registered", body = GitHubAppRegistrationResponse),
        (status = 400, description = "The App could not be proved, or the name is unusable", body = moatless_api_schemas::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_api_schemas::error::ErrorBody),
        (status = 409, description = "That name is already in use", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn admin_register_github_app(
    State(administration): State<Arc<AppAdministration>>,
    auth: AuthUser,
    Json(request): Json<RegisterGitHubAppRequest>,
) -> Result<(axum::http::StatusCode, Json<GitHubAppRegistrationResponse>), AccessSettingsError> {
    require_admin(&auth, "register")?;
    let registered = administration
        .register_github_app(
            GitHubAppRegistrationInput {
                github_app_key: request.github_app_key,
                app_id: request.app_id,
                private_key_pem: request.private_key_pem,
                default_installation_id: request.default_installation_id,
            },
            &auth.user_id.to_string(),
        )
        .await?;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(github_registration(registered)),
    ))
}

#[utoipa::path(
    put,
    path = "/api/v1/admin/settings/github/apps/{github_app_key}/key",
    tag = "app-administration",
    params(("github_app_key" = String, Path, description = "App name chosen at registration")),
    request_body = RotateGitHubAppKeyRequest,
    responses(
        (status = 200, description = "New key proved and stored", body = GitHubAppRegistrationResponse),
        (status = 400, description = "The new key could not be proved; the App is unchanged", body = moatless_api_schemas::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "No App is registered under that name", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn admin_rotate_github_app_key(
    State(administration): State<Arc<AppAdministration>>,
    auth: AuthUser,
    Path(github_app_key): Path<String>,
    Json(request): Json<RotateGitHubAppKeyRequest>,
) -> Result<Json<GitHubAppRegistrationResponse>, AccessSettingsError> {
    require_admin(&auth, "rotate a key for")?;
    Ok(Json(github_registration(
        administration
            .rotate_github_app_key(
                &github_app_key,
                request.private_key_pem,
                &auth.user_id.to_string(),
            )
            .await?,
    )))
}

#[utoipa::path(
    delete,
    path = "/api/v1/admin/settings/github/apps/{github_app_key}",
    tag = "app-administration",
    params(("github_app_key" = String, Path, description = "App name chosen at registration")),
    responses(
        (status = 200, description = "App removed installation-wide", body = RemoveGitHubAppResponse),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "No App is registered under that name", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn admin_remove_github_app(
    State(administration): State<Arc<AppAdministration>>,
    auth: AuthUser,
    Path(github_app_key): Path<String>,
) -> Result<Json<RemoveGitHubAppResponse>, AccessSettingsError> {
    require_admin(&auth, "remove")?;
    let removed = administration
        .remove_github_app(&github_app_key, &auth.user_id.to_string())
        .await?;
    Ok(Json(RemoveGitHubAppResponse {
        github_app_key: removed.github_app_key,
        orphaned_bot_users: removed.orphaned_bot_users,
    }))
}

fn github_status(status: GitHubProviderTokenStatus) -> GitHubProviderTokenStatusResponse {
    GitHubProviderTokenStatusResponse {
        connected: status.connected,
        login: status.login,
        token_type: status.token_type,
        has_pat_override: status.has_pat_override,
    }
}

fn gitness_status(status: GitnessProviderTokenStatus) -> GitnessProviderTokenStatusResponse {
    GitnessProviderTokenStatusResponse {
        connected: status.connected,
        login: status.login,
        base_url: status.base_url,
    }
}

fn agent_harness_status(
    status: AgentHarnessCredentialStatus,
) -> CodexAgentHarnessCredentialStatusResponse {
    CodexAgentHarnessCredentialStatusResponse {
        connected: status.connected,
        token_type: status.token_type,
        auth_method: status.auth_method,
        email: status.email,
        plan_type: status.plan_type,
        needs_reconnect: status.needs_reconnect,
    }
}

fn adapter_app_summary(app: crate::adapter_apps::StoredAdapterApp) -> AdapterAppSummary {
    AdapterAppSummary {
        adapter_kind: app.adapter_kind,
        app_key: app.app_key,
        display_name: app.display_name,
        config: app.config_json,
        secret_fingerprints: app.secret_fingerprints,
        revision: app.revision,
        updated_at: app.updated_at.to_rfc3339(),
    }
}

fn adapter_app_save(request: SaveAdapterAppRequest) -> AdapterAppSave {
    match request {
        SaveAdapterAppRequest::Slack(request) => AdapterAppSave::Slack {
            bot_token: request.bot_token,
            signing_secret: request.signing_secret,
            bot_user_id: request.bot_user_id,
        },
        SaveAdapterAppRequest::Telegram(request) => AdapterAppSave::Telegram {
            bot_token: request.bot_token,
            mode: request.mode,
        },
        SaveAdapterAppRequest::Linear(request) => AdapterAppSave::Linear {
            webhook_secret: request.webhook_secret,
            oauth_client_id: request.oauth_client_id,
            oauth_client_secret: request.oauth_client_secret,
            oauth_callback_url: request.oauth_callback_url,
            api_base_url: request.api_base_url,
        },
    }
}

fn github_app_option(app: GitHubAppView) -> GitHubAppOption {
    GitHubAppOption {
        github_app_key: app.github_app_key,
        name: app.name,
        install_url: app.install_url,
        installations: app
            .installations
            .into_iter()
            .map(|installation| GitHubInstallationOption {
                installation_id: installation.installation_id,
                account_login: installation.account_login,
                account_type: installation.account_type,
            })
            .collect(),
        deployment_configured: app.deployment_configured,
        key_fingerprint: app.key_fingerprint,
        bot_login: app.bot_login,
        bot_user_id: app.bot_user_id,
    }
}

fn github_registration(app: GitHubAppRegistration) -> GitHubAppRegistrationResponse {
    GitHubAppRegistrationResponse {
        github_app_key: app.github_app_key,
        app_id: app.app_id,
        name: app.name,
        slug: app.slug,
        bot_login: app.bot_login,
        key_fingerprint: app.key_fingerprint,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_admin_is_refused_before_an_administration_call_can_be_made() {
        let auth = AuthUser {
            user_id: uuid::Uuid::new_v4(),
            login: "reader".to_string(),
            provider: Some("local".to_string()),
            role: Some("user".to_string()),
            group_id: None,
            task_id: None,
            is_agent_context: false,
        };
        assert!(matches!(
            require_admin(&auth, "read"),
            Err(AccessSettingsError::Forbidden(_))
        ));
    }
}
