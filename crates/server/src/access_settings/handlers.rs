//! Thin HTTP handlers for access administration.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};

use crate::access_settings::error::{AccessSettingsError, auth_error};
use crate::access_settings::provider_apps::{self, ProviderApps};
use crate::access_settings::schemas::*;
use crate::agent_harness_access::{
    AgentHarnessAccess, AgentHarnessCredentialStatus, DeviceLoginPoll,
};
use moatless_api_schemas::provider_apps::{
    ProviderAppSummary, ProviderAppsResponse, SaveProviderAppRequest,
};
use moatless_core::http_auth::AuthUser;
use moatless_identity::adapter_apps::administration::{
    AdapterAppSave, AppAdministration, GitHubAppRegistration, GitHubAppRegistrationInput,
    GitHubAppView,
};
use moatless_identity::forgejo::{
    ForgejoAccess, ForgejoInstances, ForgejoProviderTokenStatus, RegisteredForgejoInstance,
};
use moatless_identity::git_host_access::{
    GitHostAccess, GitHostPrincipal, GitHubProviderTokenStatus, GitnessProviderTokenStatus,
};
use moatless_types::agent::CodexScope;

fn principal(auth: &AuthUser) -> GitHostPrincipal {
    GitHostPrincipal {
        user_id: auth.user_id.to_string(),
        scope_context: auth.scope_context(),
    }
}

fn require_admin(auth: &AuthUser) -> Result<(), AccessSettingsError> {
    if auth.scope_context().is_admin {
        Ok(())
    } else {
        Err(AccessSettingsError::Forbidden(
            "Only administrators can administer Forgejo instances".to_string(),
        ))
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
    path = "/api/v1/settings/forgejo/config",
    tag = "git-host-access",
    params(ForgejoHostQuery),
    responses(
        (status = 200, description = "Forgejo config for one instance", body = ForgejoProviderTokenStatusResponse),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn get_forgejo_config(
    State(access): State<Arc<ForgejoAccess>>,
    auth: AuthUser,
    Query(query): Query<ForgejoHostQuery>,
) -> Result<Json<ForgejoProviderTokenStatusResponse>, AccessSettingsError> {
    Ok(Json(forgejo_status(
        access
            .status(&auth.user_id.to_string(), &query.host)
            .await?,
    )))
}

/// Lists every registered instance with the caller's own connection to each.
///
/// Any signed-in caller may read this, unlike [`admin_list_forgejo_instances`],
/// since a hostname isn't a secret.
#[utoipa::path(
    get,
    path = "/api/v1/settings/forgejo/instances",
    tag = "git-host-access",
    responses(
        (status = 200, description = "Registered instances and the caller's connection to each", body = ForgejoConnectionsResponse),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn list_forgejo_connections(
    State(instances): State<Arc<ForgejoInstances>>,
    State(access): State<Arc<ForgejoAccess>>,
    auth: AuthUser,
) -> Result<Json<ForgejoConnectionsResponse>, AccessSettingsError> {
    let user_id = auth.user_id.to_string();
    let registered = instances.list().await.map_err(auth_error)?;
    let mut connections = Vec::with_capacity(registered.len());
    for instance in registered {
        connections.push(forgejo_status(
            access.status(&user_id, &instance.host).await?,
        ));
    }
    Ok(Json(ForgejoConnectionsResponse {
        instances: connections,
    }))
}

#[utoipa::path(
    put,
    path = "/api/v1/settings/forgejo/pat",
    tag = "git-host-access",
    request_body = SaveForgejoProviderTokenRequest,
    responses(
        (status = 200, description = "PAT saved", body = ForgejoProviderTokenStatusResponse),
        (status = 400, description = "The instance rejected the token", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn save_forgejo_pat(
    State(access): State<Arc<ForgejoAccess>>,
    auth: AuthUser,
    Json(request): Json<SaveForgejoProviderTokenRequest>,
) -> Result<Json<ForgejoProviderTokenStatusResponse>, AccessSettingsError> {
    Ok(Json(forgejo_status(
        access
            .save_personal_access_token(&principal(&auth), &request.base_url, request.token)
            .await?,
    )))
}

#[utoipa::path(
    delete,
    path = "/api/v1/settings/forgejo/pat",
    tag = "git-host-access",
    params(ForgejoHostQuery),
    responses(
        (status = 200, description = "PAT override removed", body = ForgejoProviderTokenStatusResponse),
        (status = 400, description = "No OAuth credential was preserved", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn delete_forgejo_pat_override(
    State(access): State<Arc<ForgejoAccess>>,
    auth: AuthUser,
    Query(query): Query<ForgejoHostQuery>,
) -> Result<Json<ForgejoProviderTokenStatusResponse>, AccessSettingsError> {
    Ok(Json(forgejo_status(
        access
            .restore_oauth_token(&auth.user_id.to_string(), &query.host)
            .await?,
    )))
}

#[utoipa::path(
    delete,
    path = "/api/v1/settings/forgejo/config",
    tag = "git-host-access",
    params(ForgejoHostQuery),
    responses(
        (status = 200, description = "Instance disconnected", body = ForgejoProviderTokenStatusResponse),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn delete_forgejo_config(
    State(access): State<Arc<ForgejoAccess>>,
    auth: AuthUser,
    Query(query): Query<ForgejoHostQuery>,
) -> Result<Json<ForgejoProviderTokenStatusResponse>, AccessSettingsError> {
    Ok(Json(forgejo_status(
        access
            .disconnect(&auth.user_id.to_string(), &query.host)
            .await?,
    )))
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/forgejo-instances",
    tag = "app-administration",
    responses(
        (status = 200, description = "Registered instances", body = ForgejoInstancesResponse),
        (status = 403, description = "Admin only", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn admin_list_forgejo_instances(
    State(instances): State<Arc<ForgejoInstances>>,
    auth: AuthUser,
) -> Result<Json<ForgejoInstancesResponse>, AccessSettingsError> {
    require_admin(&auth)?;
    Ok(Json(ForgejoInstancesResponse {
        instances: instances
            .list()
            .await
            .map_err(auth_error)?
            .into_iter()
            .map(forgejo_instance_summary)
            .collect(),
    }))
}

#[utoipa::path(
    put,
    path = "/api/v1/admin/forgejo-instances/{host}",
    tag = "app-administration",
    params(("host" = String, Path, description = "The instance's hostname")),
    request_body = RegisterForgejoInstanceRequest,
    responses(
        (status = 200, description = "Application registered", body = ForgejoInstanceSummary),
        (status = 400, description = "The application cannot be stored", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn admin_register_forgejo_instance(
    State(instances): State<Arc<ForgejoInstances>>,
    auth: AuthUser,
    Path(host): Path<String>,
    Json(request): Json<RegisterForgejoInstanceRequest>,
) -> Result<Json<ForgejoInstanceSummary>, AccessSettingsError> {
    require_admin(&auth)?;
    let registered = instances
        .register(
            &host,
            &request.client_id,
            &request.client_secret,
            &auth.user_id.to_string(),
        )
        .await
        .map_err(auth_error)?;
    Ok(Json(forgejo_instance_summary(registered)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/admin/forgejo-instances/{host}",
    tag = "app-administration",
    params(("host" = String, Path, description = "The instance's hostname")),
    responses(
        (status = 204, description = "Application removed"),
        (status = 403, description = "Admin only", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "No application is registered for that host", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn admin_remove_forgejo_instance(
    State(instances): State<Arc<ForgejoInstances>>,
    auth: AuthUser,
    Path(host): Path<String>,
) -> Result<axum::http::StatusCode, AccessSettingsError> {
    require_admin(&auth)?;
    if instances.remove(&host).await.map_err(auth_error)? {
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Err(AccessSettingsError::NotFound(format!(
            "No Forgejo OAuth application is registered for {host}"
        )))
    }
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
            .status(
                &auth.scope_context(),
                &CodexScope::user(auth.user_id.to_string()),
            )
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
                &auth.scope_context(),
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
            .delete(
                &auth.scope_context(),
                &CodexScope::user(auth.user_id.to_string()),
            )
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
    Ok(Json(agent_harness_status(
        access
            .save(&auth.scope_context(), CodexScope::Global, request.auth_json)
            .await?,
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
    Ok(Json(agent_harness_status(
        access
            .delete(&auth.scope_context(), &CodexScope::Global)
            .await?,
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
    Ok(Json(agent_harness_status(
        access
            .save_for_user(&auth.scope_context(), &target_user_id, request.auth_json)
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
            &auth.scope_context(),
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
    path = "/api/v1/admin/settings/apps",
    tag = "app-administration",
    responses(
        (status = 200, description = "Every provider spec, and every app registered", body = ProviderAppsResponse),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn admin_list_provider_apps(
    State(apps): State<ProviderApps>,
    auth: AuthUser,
) -> Result<Json<ProviderAppsResponse>, AccessSettingsError> {
    Ok(Json(ProviderAppsResponse {
        specs: provider_apps::specs(&apps.frontend_url),
        apps: provider_apps::list(&apps, &auth.scope_context()).await?,
    }))
}

#[utoipa::path(
    put,
    path = "/api/v1/admin/settings/apps/{provider}/{key}",
    tag = "app-administration",
    params(
        ("provider" = String, Path, description = "A provider named by GET /api/v1/admin/settings/apps"),
        ("key" = String, Path, description = "A Forgejo host, or an adapter's app key")
    ),
    request_body = SaveProviderAppRequest,
    responses(
        (status = 200, description = "App registered", body = ProviderAppSummary),
        (status = 400, description = "A field is unknown, missing, or the provider rejected the credentials", body = moatless_api_schemas::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "No such provider", body = moatless_api_schemas::error::ErrorBody),
        (status = 502, description = "The provider could not be reached to check the credentials", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn admin_save_provider_app(
    State(apps): State<ProviderApps>,
    auth: AuthUser,
    Path((provider, key)): Path<(String, String)>,
    Json(request): Json<SaveProviderAppRequest>,
) -> Result<Json<ProviderAppSummary>, AccessSettingsError> {
    Ok(Json(
        provider_apps::save(
            &apps,
            &auth.scope_context(),
            &provider,
            &key,
            request.fields,
        )
        .await?,
    ))
}

#[utoipa::path(
    delete,
    path = "/api/v1/admin/settings/apps/{provider}/{key}",
    tag = "app-administration",
    params(
        ("provider" = String, Path, description = "A provider named by GET /api/v1/admin/settings/apps"),
        ("key" = String, Path, description = "A Forgejo host, or an adapter's app key")
    ),
    responses(
        (status = 204, description = "App removed"),
        (status = 401, description = "Not authenticated", body = moatless_api_schemas::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_api_schemas::error::ErrorBody),
        (status = 404, description = "Nothing is registered under that provider and key", body = moatless_api_schemas::error::ErrorBody)
    )
)]
pub async fn admin_remove_provider_app(
    State(apps): State<ProviderApps>,
    auth: AuthUser,
    Path((provider, key)): Path<(String, String)>,
) -> Result<axum::http::StatusCode, AccessSettingsError> {
    if provider_apps::remove(&apps, &auth.scope_context(), &provider, &key).await? {
        Ok(axum::http::StatusCode::NO_CONTENT)
    } else {
        Err(AccessSettingsError::NotFound(format!(
            "No {provider} app is registered as '{key}'"
        )))
    }
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
    Ok(Json(AdapterAppsResponse {
        apps: administration
            .list_adapter_apps(&auth.scope_context())
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
    Ok(Json(adapter_app_summary(
        administration
            .save_adapter_app(
                &adapter_kind,
                &app_key,
                adapter_app_save(request),
                &auth.scope_context(),
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
    administration
        .delete_adapter_app(&auth.scope_context(), &adapter_kind, &app_key)
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
    Ok(Json(GitHubAppsResponse {
        apps: administration
            .list_github_apps(&auth.scope_context())
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
    let registered = administration
        .register_github_app(
            GitHubAppRegistrationInput {
                github_app_key: request.github_app_key,
                app_id: request.app_id,
                private_key_pem: request.private_key_pem,
                default_installation_id: request.default_installation_id,
            },
            &auth.scope_context(),
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
    Ok(Json(github_registration(
        administration
            .rotate_github_app_key(
                &github_app_key,
                request.private_key_pem,
                &auth.scope_context(),
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
    let removed = administration
        .remove_github_app(&github_app_key, &auth.scope_context())
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
        auth_method: status.auth_method,
        has_pat_override: status.has_pat_override,
        app_connect_available: status.app_connect_available,
    }
}

fn gitness_status(status: GitnessProviderTokenStatus) -> GitnessProviderTokenStatusResponse {
    GitnessProviderTokenStatusResponse {
        connected: status.connected,
        login: status.login,
        base_url: status.base_url,
    }
}

fn forgejo_status(status: ForgejoProviderTokenStatus) -> ForgejoProviderTokenStatusResponse {
    ForgejoProviderTokenStatusResponse {
        host: status.host,
        connected: status.connected,
        auth_method: status.auth_method,
        has_pat_override: status.has_pat_override,
        oauth_available: status.oauth_available,
    }
}

fn forgejo_instance_summary(instance: RegisteredForgejoInstance) -> ForgejoInstanceSummary {
    ForgejoInstanceSummary {
        host: instance.host,
        client_id: instance.client_id,
        updated_at: instance.updated_at.to_rfc3339(),
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

fn adapter_app_summary(
    app: moatless_identity::adapter_apps::StoredAdapterApp,
) -> AdapterAppSummary {
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
            moatless_identity::adapter_apps::administration::require_admin(
                &auth.scope_context(),
                "read"
            )
            .map_err(AccessSettingsError::from),
            Err(AccessSettingsError::Forbidden(_))
        ));
    }
}
