//! Provider settings HTTP handlers (GitHub, Gitness, Codex).

use std::collections::HashMap;
use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};

use moatless_backend_core::scope::{Scope, ScopeContext};

use crate::adapter_apps::dao::{AdapterAppDao, GitHubAppAuditEntry, GitHubAppRecord, NewGitHubApp};
use crate::adapter_apps::{
    AdapterAppInput, AdapterAppSecretInput, DEFAULT_APP_KEY, delete_adapter_app, list_adapter_apps,
    upsert_adapter_app,
};
use crate::adapters::dao::AdapterConnectionDao;
use crate::adapters::models::AdapterConnection;
use crate::auth::codex_scope::CodexScope;
use crate::auth::credentials::{
    GITHUB_OAUTH_STASH_KEY, read_github_oauth_stash, stash_github_oauth_token,
};
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::service::GitHubAppService;
use crate::auth::github_scope::GitHubScope;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::{
    APP_INSTALLATION_TOKEN_PLACEHOLDER, AUTH_METHOD_APP_INSTALLATION, AUTH_METHOD_SECRET_REF,
    SECRET_REF_TOKEN_PLACEHOLDER,
};
use crate::credential_sync::CredentialSync;
use crate::credentials::codex::oauth::{self as codex_oauth, PollStatus};
use crate::extractors::AuthUser;
use crate::provider_settings::codex_save::save_codex_auth_for_scope;
use crate::secret::models::SecretKind;
use crate::secret::schemas::PutSecretRequest;
use crate::secret::service::{SecretCipher, SecretService};
use moatless_core::config::{CodexOAuthConfig, SecretsConfig};
use moatless_core::db::DbPool;
use moatless_core::secret_keys::GITHUB_TOKEN;

use super::error::ProviderSettingsError;
use super::gitness_client;
use super::schemas::*;
use moatless_core::github_client;

/// Get current GitHub configuration for the user.
#[utoipa::path(
    get,
    path = "/api/v1/settings/github/config",
    tag = "provider-settings",
    responses(
        (status = 200, description = "GitHub config", body = GitHubConfigResponse),
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn get_github_config(
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    auth: AuthUser,
) -> Result<Json<GitHubConfigResponse>, ProviderSettingsError> {
    let token = provider_tokens
        .find_provider_token_full(&auth.user_id.to_string(), "github")
        .await?;

    let is_github_app_user = auth.provider.as_deref() == Some("github_app");

    match token {
        Some(row) => {
            let login = row
                .metadata
                .as_ref()
                .and_then(|m| m.get("login"))
                .and_then(|v| v.as_str())
                .map(std::string::ToString::to_string);

            let has_pat_override = if is_github_app_user {
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
            has_pat_override: if is_github_app_user {
                Some(false)
            } else {
                None
            },
        })),
    }
}

// ============================================================================
// Repository-scoped GitHub App installation bindings — admin only
// ============================================================================

/// `adapter_kind` used for GitHub rows in `adapter_connections` (matches
/// the webhook ingress and binding resolution lookups).
const GITHUB_ADAPTER_KIND: &str = "github";

fn require_admin(auth: &AuthUser, action: &str) -> Result<(), ProviderSettingsError> {
    if auth.role.as_deref() == Some("admin") {
        return Ok(());
    }
    Err(ProviderSettingsError::Forbidden(format!(
        "Admin role required to {action} provider settings"
    )))
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

fn adapter_app_input(
    adapter_kind: &str,
    app_key: &str,
    req: SaveAdapterAppRequest,
) -> Result<AdapterAppInput, ProviderSettingsError> {
    if adapter_kind == "github" {
        return Err(ProviderSettingsError::BadRequest(
            "Use the GitHub App admin endpoints for GitHub App registration".to_string(),
        ));
    }
    match req {
        SaveAdapterAppRequest::Slack(slack) => {
            if adapter_kind != "slack" {
                return Err(ProviderSettingsError::BadRequest(
                    "Request body adapterKind must match the path".to_string(),
                ));
            }
            Ok(AdapterAppInput {
                adapter_kind: "slack",
                app_key: app_key.to_string(),
                display_name: "Slack".to_string(),
                config_json: serde_json::json!({ "bot_user_id": slack.bot_user_id }),
                secrets: vec![
                    AdapterAppSecretInput {
                        name: "bot_token",
                        plaintext: slack.bot_token,
                    },
                    AdapterAppSecretInput {
                        name: "signing_secret",
                        plaintext: slack.signing_secret,
                    },
                ],
            })
        }
        SaveAdapterAppRequest::Telegram(telegram) => {
            if adapter_kind != "telegram" {
                return Err(ProviderSettingsError::BadRequest(
                    "Request body adapterKind must match the path".to_string(),
                ));
            }
            let mode = telegram.mode.unwrap_or_else(|| "webhook".to_string());
            if mode != "webhook" && mode != "polling" {
                return Err(ProviderSettingsError::BadRequest(
                    "Telegram mode must be 'webhook' or 'polling'".to_string(),
                ));
            }
            Ok(AdapterAppInput {
                adapter_kind: "telegram",
                app_key: app_key.to_string(),
                display_name: "Telegram".to_string(),
                config_json: serde_json::json!({ "mode": mode }),
                secrets: vec![AdapterAppSecretInput {
                    name: "bot_token",
                    plaintext: telegram.bot_token,
                }],
            })
        }
        SaveAdapterAppRequest::Linear(linear) => {
            if adapter_kind != "linear" {
                return Err(ProviderSettingsError::BadRequest(
                    "Request body adapterKind must match the path".to_string(),
                ));
            }
            let mut secrets = Vec::new();
            if let Some(value) = linear.webhook_secret {
                secrets.push(AdapterAppSecretInput {
                    name: "webhook_secret",
                    plaintext: value,
                });
            }
            if let Some(value) = linear.oauth_client_id {
                secrets.push(AdapterAppSecretInput {
                    name: "oauth_client_id",
                    plaintext: value,
                });
            }
            if let Some(value) = linear.oauth_client_secret {
                secrets.push(AdapterAppSecretInput {
                    name: "oauth_client_secret",
                    plaintext: value,
                });
            }
            Ok(AdapterAppInput {
                adapter_kind: "linear",
                app_key: app_key.to_string(),
                display_name: "Linear".to_string(),
                config_json: serde_json::json!({
                    "oauth_callback_url": linear.oauth_callback_url,
                    "api_base_url": linear.api_base_url,
                }),
                secrets,
            })
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/settings/adapter-apps",
    tag = "provider-settings",
    responses(
        (status = 200, description = "Configured Adapter apps", body = AdapterAppsResponse),
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn admin_list_adapter_apps(
    State(db): State<DbPool>,
    auth: AuthUser,
) -> Result<Json<AdapterAppsResponse>, ProviderSettingsError> {
    require_admin(&auth, "read")?;
    let apps = list_adapter_apps(&db)
        .await?
        .into_iter()
        .map(adapter_app_summary)
        .collect();
    Ok(Json(AdapterAppsResponse { apps }))
}

#[utoipa::path(
    put,
    path = "/api/v1/admin/settings/adapter-apps/{adapter_kind}/{app_key}",
    tag = "provider-settings",
    params(
        ("adapter_kind" = String, Path, description = "Adapter kind"),
        ("app_key" = String, Path, description = "Adapter app key")
    ),
    request_body = SaveAdapterAppRequest,
    responses(
        (status = 200, description = "Adapter app saved", body = AdapterAppSummary),
        (status = 400, description = "Invalid Adapter app config", body = moatless_core::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn admin_save_adapter_app(
    State(db): State<DbPool>,
    State(secrets_config): State<Arc<SecretsConfig>>,
    auth: AuthUser,
    Path((adapter_kind, app_key)): Path<(String, String)>,
    Json(req): Json<SaveAdapterAppRequest>,
) -> Result<Json<AdapterAppSummary>, ProviderSettingsError> {
    require_admin(&auth, "save")?;
    if app_key != DEFAULT_APP_KEY {
        return Err(ProviderSettingsError::BadRequest(format!(
            "Only the '{DEFAULT_APP_KEY}' Adapter app key is supported for this Adapter"
        )));
    }
    let input = adapter_app_input(&adapter_kind, &app_key, req)?;
    let app = upsert_adapter_app(&db, &secrets_config, input, &auth.user_id.to_string()).await?;
    Ok(Json(adapter_app_summary(app)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/admin/settings/adapter-apps/{adapter_kind}/{app_key}",
    tag = "provider-settings",
    params(
        ("adapter_kind" = String, Path, description = "Adapter kind"),
        ("app_key" = String, Path, description = "Adapter app key")
    ),
    responses(
        (status = 204, description = "Adapter app deleted"),
        (status = 400, description = "Invalid Adapter app", body = moatless_core::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn admin_delete_adapter_app(
    State(db): State<DbPool>,
    auth: AuthUser,
    Path((adapter_kind, app_key)): Path<(String, String)>,
) -> Result<axum::http::StatusCode, ProviderSettingsError> {
    require_admin(&auth, "delete")?;
    if adapter_kind == "github" {
        return Err(ProviderSettingsError::BadRequest(
            "Use the GitHub App admin endpoints to remove GitHub Apps".to_string(),
        ));
    }
    delete_adapter_app(&db, &adapter_kind, &app_key).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

fn installation_binding_response(
    row: Option<crate::auth::models::ProviderToken>,
) -> GitHubInstallationBindingResponse {
    let Some(row) = row else {
        return GitHubInstallationBindingResponse {
            configured: false,
            github_app_key: None,
            installation_id: None,
            account_login: None,
            account_type: None,
        };
    };
    let installation_id = row
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("installation_id"))
        .and_then(serde_json::Value::as_u64);
    let account_login = row
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("account_login"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let account_type = row
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get("account_type"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let github_app_key = row.provider_user_id;
    let configured = row.auth_method.as_deref() == Some(AUTH_METHOD_APP_INSTALLATION)
        && github_app_key.is_some()
        && installation_id.is_some()
        && account_login.is_some();

    GitHubInstallationBindingResponse {
        configured,
        github_app_key,
        installation_id,
        account_login,
        account_type,
    }
}

async fn save_installation_binding(
    github_apps: &Arc<GitHubAppRegistry>,
    provider_tokens: &Arc<dyn ProviderTokenDao>,
    adapter_connections: &Arc<dyn AdapterConnectionDao>,
    ctx: &ScopeContext,
    scope: &GitHubScope,
    req: GitHubInstallationBindingRequest,
    created_by: &str,
) -> Result<GitHubInstallationBindingResponse, ProviderSettingsError> {
    let app = github_apps.require(&req.github_app_key).await?;
    let installation = app
        .get_installation(req.installation_id)
        .await
        .map_err(ProviderSettingsError::GitHubApp)?;
    let metadata = serde_json::json!({
        "installation_id": installation.id,
        "account_login": installation.account.login,
        "account_type": installation.account.account_type,
    });

    provider_tokens
        .upsert_github_row(
            scope,
            AUTH_METHOD_APP_INSTALLATION,
            APP_INSTALLATION_TOKEN_PLACEHOLDER,
            None,
            Some(&metadata),
            Some(&req.github_app_key),
            Some(&installation.account.login),
        )
        .await?;

    upsert_github_org_connection(
        adapter_connections,
        ctx,
        scope,
        &installation.account.login,
        &req.github_app_key,
        installation.id,
        created_by,
    )
    .await?;

    Ok(installation_binding_response(
        provider_tokens.find_github_row(scope).await?,
    ))
}

/// Mirror an installation binding into `adapter_connections` so inbound
/// GitHub webhooks and task↔PR bindings resolve for the installation's
/// account (`external_account_id` = org/user login). Without this row,
/// `resolve_connection` rejects the org and PRs created from tasks never
/// show up as task connections.
///
/// An existing row for the same account keeps its id, webhook secret,
/// credentials and connection kind; only the scope is retargeted to the
/// binding's team (or global). Unbinding leaves the row in place because
/// task bindings may still reference it.
async fn upsert_github_org_connection(
    adapter_connections: &Arc<dyn AdapterConnectionDao>,
    ctx: &ScopeContext,
    scope: &GitHubScope,
    account_login: &str,
    github_app_key: &str,
    installation_id: u64,
    created_by: &str,
) -> Result<(), ProviderSettingsError> {
    // Installation bindings are only exposed at global scope.
    if !matches!(scope, GitHubScope::Global) {
        return Ok(());
    }

    let repo = adapter_connections;
    let existing = repo
        .find_by_external_account(GITHUB_ADAPTER_KIND, account_login)
        .await?;

    let now = chrono::Utc::now();
    let mut transport_config = existing
        .as_ref()
        .map(|c| c.transport_config.clone())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    if let Some(fields) = transport_config.as_object_mut() {
        fields.insert("github_app_key".to_string(), github_app_key.into());
        fields.insert(
            "installation_id".to_string(),
            serde_json::Value::from(installation_id),
        );
    }

    let row = AdapterConnection {
        id: existing
            .as_ref()
            .map_or_else(|| format!("pc_{}", uuid::Uuid::new_v4()), |c| c.id.clone()),
        adapter_kind: GITHUB_ADAPTER_KIND.to_string(),
        connection_kind: existing
            .as_ref()
            .map_or_else(|| "org_webhook".to_string(), |c| c.connection_kind.clone()),
        scope_value: account_login.to_string(),
        credential_ref: existing.as_ref().and_then(|c| c.credential_ref.clone()),
        webhook_secret: existing.as_ref().and_then(|c| c.webhook_secret.clone()),
        transport_config,
        created_by: existing
            .as_ref()
            .map_or_else(|| created_by.to_string(), |c| c.created_by.clone()),
        scope: Scope::Global,
        owner_user_id: None,
        created_at: existing.as_ref().map_or(now, |c| c.created_at),
        updated_at: now,
    };
    repo.upsert(&row, ctx).await?;
    Ok(())
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/settings/github/apps",
    tag = "provider-settings",
    responses(
        (status = 200, description = "Configured GitHub Apps and installations", body = GitHubAppsResponse),
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn admin_list_github_apps(
    State(github_apps): State<Arc<GitHubAppRegistry>>,
    State(db): State<DbPool>,
    auth: AuthUser,
) -> Result<Json<GitHubAppsResponse>, ProviderSettingsError> {
    require_admin(&auth, "read")?;

    let entries = github_apps.entries().await?;
    // Whatever the registry holds that storage does not know about is the
    // deployment-configured login App — the one row this page cannot edit.
    let stored_apps: HashMap<String, GitHubAppRecord> = match github_apps.store() {
        Some(store) => store
            .list_github_apps()
            .await?
            .into_iter()
            .map(|record| (record.app_key.clone(), record))
            .collect(),
        None => HashMap::new(),
    };
    let bot_users = bot_users_by_app_key(&db).await?;

    let mut apps = Vec::new();
    for (github_app_key, app) in entries {
        let (name, install_url) = app
            .app_listing()
            .await
            .map_err(ProviderSettingsError::GitHubApp)?;
        let installations = app
            .list_installations()
            .await
            .map_err(ProviderSettingsError::GitHubApp)?
            .into_iter()
            .map(|installation| GitHubInstallationOption {
                installation_id: installation.id,
                account_login: installation.account.login,
                account_type: installation.account.account_type,
            })
            .collect();
        let stored = stored_apps.get(&github_app_key);
        apps.push(GitHubAppOption {
            github_app_key: github_app_key.clone(),
            name,
            install_url,
            installations,
            deployment_configured: stored.is_none(),
            key_fingerprint: stored.map(|record| record.key_fingerprint.clone()),
            bot_login: stored.map(|record| record.bot_login.clone()),
            bot_user_id: bot_users.get(&github_app_key).cloned(),
        });
    }
    Ok(Json(GitHubAppsResponse { apps }))
}

/// Bot user id per App key, for every App that has been staffed.
///
/// Bot user creation writes the App key into the user's provider identity as
/// `bot:<key>`, so this is the whole relation — there is no join to make.
async fn bot_users_by_app_key(
    db: &moatless_core::db::DbPool,
) -> Result<HashMap<String, String>, ProviderSettingsError> {
    let rows: Vec<(uuid::Uuid, String)> = sqlx::query_as(
        "SELECT id, provider_user_id FROM users \
         WHERE is_bot = true AND provider_user_id LIKE 'bot:%'",
    )
    .fetch_all(db)
    .await?;
    Ok(rows
        .into_iter()
        .filter_map(|(id, provider_user_id)| {
            provider_user_id
                .strip_prefix("bot:")
                .map(|app_key| (app_key.to_string(), id.to_string()))
        })
        .collect())
}

/// The durable store and the cipher an App's signing key is sealed with.
type AppAdministration<'a> = (&'a Arc<dyn AdapterAppDao>, &'a Arc<dyn SecretCipher>);

/// Everything needed to register or rotate an App: the durable store and the
/// cipher its signing key is sealed with.
///
/// Both are absent on an installation with no database or no encryption
/// configured. Refusing here, by name, is the whole answer to open question 1:
/// such an installation starts and serves normally, and says plainly why it
/// cannot store an App the moment someone tries.
fn app_administration(
    github_apps: &Arc<GitHubAppRegistry>,
) -> Result<AppAdministration<'_>, ProviderSettingsError> {
    let registry = github_apps;
    let (Some(store), Some(cipher)) = (registry.store(), registry.cipher()) else {
        return Err(ProviderSettingsError::BadRequest(
            "This installation cannot store GitHub Apps: it has no encryption configured. \
             Set SECRETS_ENCRYPTION_KEY and try again."
                .to_string(),
        ));
    };
    Ok((store, cipher))
}

/// The App key names the App everywhere — in bot user identities, in Adapter
/// connection configuration, in Loop configuration — so it is constrained to
/// what all of those can carry, and matches the storage constraint exactly.
fn validate_app_key(app_key: &str) -> Result<(), ProviderSettingsError> {
    if app_key == GitHubAppService::DEFAULT_KEY {
        return Err(ProviderSettingsError::BadRequest(format!(
            "'{}' names the App this deployment signs in with and cannot be registered",
            GitHubAppService::DEFAULT_KEY
        )));
    }
    let valid = !app_key.is_empty()
        && app_key.len() <= 64
        && app_key.starts_with(|c: char| c.is_ascii_alphanumeric())
        && app_key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid {
        return Err(ProviderSettingsError::BadRequest(format!(
            "'{app_key}' is not a usable App name: use letters, digits, '-' and '_', \
             starting with a letter or digit, at most 64 characters"
        )));
    }
    Ok(())
}

/// Prove a signing key against GitHub, as the App id given.
///
/// Nothing is stored until this returns. The messages are the admin-facing
/// ones: which part failed, and that nothing was kept.
async fn prove_signing_key(
    app_key: &str,
    app_id: u64,
    private_key_pem: &str,
    default_installation_id: Option<u64>,
) -> Result<crate::auth::github::service::ProvedApp, ProviderSettingsError> {
    let entry = moatless_core::config::GitHubAppConfig {
        key: app_key.to_string(),
        app_id,
        private_key: private_key_pem.to_string(),
        client_id: None,
        client_secret: None,
        default_installation_id,
    };
    let service = GitHubAppService::from_app_config(&entry).ok_or_else(|| {
        ProviderSettingsError::BadRequest(
            "The signing key could not be read as a private key. Nothing was stored.".to_string(),
        )
    })?;
    Ok(service.prove().await?)
}

#[utoipa::path(
    post,
    path = "/api/v1/admin/settings/github/apps",
    tag = "provider-settings",
    request_body = RegisterGitHubAppRequest,
    responses(
        (status = 201, description = "App proved against GitHub and registered", body = GitHubAppRegistrationResponse),
        (status = 400, description = "The App could not be proved, or the name is unusable", body = moatless_core::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_core::error::ErrorBody),
        (status = 409, description = "That name is already in use", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn admin_register_github_app(
    State(github_apps): State<Arc<GitHubAppRegistry>>,
    auth: AuthUser,
    Json(req): Json<RegisterGitHubAppRequest>,
) -> Result<(axum::http::StatusCode, Json<GitHubAppRegistrationResponse>), ProviderSettingsError> {
    require_admin(&auth, "register")?;
    let (store, cipher) = app_administration(&github_apps)?;
    let actor = auth.user_id.to_string();
    validate_app_key(&req.github_app_key)
        .map_err(|e| audit_refusal(store, &req.github_app_key, &actor, "register", e))?;

    let proved = match prove_signing_key(
        &req.github_app_key,
        req.app_id,
        &req.private_key_pem,
        req.default_installation_id,
    )
    .await
    {
        Ok(proved) => proved,
        Err(e) => {
            return Err(audit_refusal(
                store,
                &req.github_app_key,
                &actor,
                "register",
                e,
            ));
        }
    };

    let record = GitHubAppRecord::seal(
        NewGitHubApp {
            app_key: req.github_app_key.clone(),
            app_id: proved.app_id,
            name: proved.name.clone(),
            slug: proved.slug.clone(),
            bot_login: proved.bot_login.clone(),
            private_key_pem: req.private_key_pem,
            default_installation_id: req.default_installation_id,
        },
        cipher.as_ref(),
        None,
    )?;
    if let Err(e) = store.insert_github_app(&record, &actor).await {
        return Err(audit_refusal(
            store,
            &req.github_app_key,
            &actor,
            "register",
            e.into(),
        ));
    }
    record_app_audit(
        store.as_ref(),
        Some(&record.id),
        &req.github_app_key,
        &actor,
        "register",
        "ok",
        None,
    )
    .await;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(GitHubAppRegistrationResponse {
            github_app_key: req.github_app_key,
            app_id: proved.app_id,
            name: proved.name,
            slug: proved.slug,
            bot_login: proved.bot_login,
            key_fingerprint: record.key_fingerprint,
        }),
    ))
}

#[utoipa::path(
    put,
    path = "/api/v1/admin/settings/github/apps/{github_app_key}/key",
    tag = "provider-settings",
    params(("github_app_key" = String, Path, description = "App name chosen at registration")),
    request_body = RotateGitHubAppKeyRequest,
    responses(
        (status = 200, description = "New key proved and stored", body = GitHubAppRegistrationResponse),
        (status = 400, description = "The new key could not be proved; the App is unchanged", body = moatless_core::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_core::error::ErrorBody),
        (status = 404, description = "No App is registered under that name", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn admin_rotate_github_app_key(
    State(github_apps): State<Arc<GitHubAppRegistry>>,
    auth: AuthUser,
    axum::extract::Path(github_app_key): axum::extract::Path<String>,
    Json(req): Json<RotateGitHubAppKeyRequest>,
) -> Result<Json<GitHubAppRegistrationResponse>, ProviderSettingsError> {
    require_admin(&auth, "rotate a key for")?;
    let (store, cipher) = app_administration(&github_apps)?;
    let actor = auth.user_id.to_string();

    let existing = store
        .find_github_app_by_key(&github_app_key)
        .await?
        .ok_or_else(|| not_registered(&github_app_key))?;

    // Proved as the App id already on record: a rotation replaces a key, never
    // repoints the name at a different App.
    let proved = match prove_signing_key(
        &github_app_key,
        existing.app_id,
        &req.private_key_pem,
        existing.default_installation_id,
    )
    .await
    {
        Ok(proved) => proved,
        Err(e) => return Err(audit_refusal(store, &github_app_key, &actor, "rotate", e)),
    };

    let record = GitHubAppRecord::seal(
        NewGitHubApp {
            app_key: github_app_key.clone(),
            app_id: proved.app_id,
            name: proved.name.clone(),
            slug: proved.slug.clone(),
            bot_login: proved.bot_login.clone(),
            private_key_pem: req.private_key_pem,
            default_installation_id: existing.default_installation_id,
        },
        cipher.as_ref(),
        Some(existing.id.clone()),
    )?;
    if !store.replace_github_app(&record, &actor).await? {
        return Err(not_registered(&github_app_key));
    }
    // This backend would otherwise serve its own tokens from the old key until
    // it next noticed the revision move.
    github_apps.forget(&github_app_key).await;
    record_app_audit(
        store.as_ref(),
        Some(&record.id),
        &github_app_key,
        &actor,
        "rotate",
        "ok",
        None,
    )
    .await;

    Ok(Json(GitHubAppRegistrationResponse {
        github_app_key,
        app_id: proved.app_id,
        name: proved.name,
        slug: proved.slug,
        bot_login: proved.bot_login,
        key_fingerprint: record.key_fingerprint,
    }))
}

#[utoipa::path(
    delete,
    path = "/api/v1/admin/settings/github/apps/{github_app_key}",
    tag = "provider-settings",
    params(("github_app_key" = String, Path, description = "App name chosen at registration")),
    responses(
        (status = 200, description = "App removed installation-wide", body = RemoveGitHubAppResponse),
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_core::error::ErrorBody),
        (status = 404, description = "No App is registered under that name", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn admin_remove_github_app(
    State(github_apps): State<Arc<GitHubAppRegistry>>,
    State(db): State<DbPool>,
    auth: AuthUser,
    axum::extract::Path(github_app_key): axum::extract::Path<String>,
) -> Result<Json<RemoveGitHubAppResponse>, ProviderSettingsError> {
    require_admin(&auth, "remove")?;
    let (store, _) = app_administration(&github_apps)?;
    let actor = auth.user_id.to_string();

    let existing = store
        .find_github_app_by_key(&github_app_key)
        .await?
        .ok_or_else(|| not_registered(&github_app_key))?;

    // Bot users are left in place: past work is attributed to them, and
    // deleting the identity would make that history unreadable. What they lose
    // is the ability to obtain a token, so they are named in the response
    // rather than quietly broken.
    let orphaned_bot_users: Vec<String> = bot_users_by_app_key(&db)
        .await?
        .remove(&github_app_key)
        .into_iter()
        .collect();

    if !store.delete_github_app(&github_app_key).await? {
        return Err(not_registered(&github_app_key));
    }
    github_apps.forget(&github_app_key).await;
    record_app_audit(
        store.as_ref(),
        Some(&existing.id),
        &github_app_key,
        &actor,
        "remove",
        "ok",
        (!orphaned_bot_users.is_empty())
            .then(|| {
                format!(
                    "{} bot user(s) left without a key",
                    orphaned_bot_users.len()
                )
            })
            .as_deref(),
    )
    .await;

    Ok(Json(RemoveGitHubAppResponse {
        github_app_key,
        orphaned_bot_users,
    }))
}

fn not_registered(app_key: &str) -> ProviderSettingsError {
    ProviderSettingsError::NotFound(format!("No GitHub App is registered as '{app_key}'"))
}

/// Record a refused administration attempt and hand the error back unchanged.
///
/// A refusal is as much a thing an admin did as a success, and the failures
/// worth reviewing later — a key that GitHub would not accept, a name already
/// taken — only exist as refusals.
fn audit_refusal(
    store: &Arc<dyn AdapterAppDao>,
    app_key: &str,
    actor_id: &str,
    action: &str,
    error: ProviderSettingsError,
) -> ProviderSettingsError {
    let store = Arc::clone(store);
    let (app_key, actor_id, action, detail) = (
        app_key.to_string(),
        actor_id.to_string(),
        action.to_string(),
        error.to_string(),
    );
    tokio::spawn(async move {
        record_app_audit(
            store.as_ref(),
            None,
            &app_key,
            &actor_id,
            &action,
            "refused",
            Some(&detail),
        )
        .await;
    });
    error
}

/// Write an audit row, or log why it could not be written.
///
/// A failed audit write never fails the operation it describes: the App has
/// already been stored or refused by this point, and turning a bookkeeping
/// failure into a 500 would leave the admin unsure which happened.
async fn record_app_audit(
    store: &dyn AdapterAppDao,
    github_app_id: Option<&str>,
    app_key: &str,
    actor_id: &str,
    action: &str,
    result: &str,
    detail: Option<&str>,
) {
    let entry = GitHubAppAuditEntry {
        github_app_id,
        app_key,
        actor_id,
        action,
        result,
        detail,
    };
    if let Err(e) = store.record_github_app_audit(entry).await {
        tracing::warn!(
            app_key = %app_key,
            action = %action,
            error = %e,
            "Could not record GitHub App audit event"
        );
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/admin/settings/github/installation",
    tag = "provider-settings",
    responses(
        (status = 200, description = "Global GitHub installation binding", body = GitHubInstallationBindingResponse),
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn admin_get_global_github_installation(
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    auth: AuthUser,
) -> Result<Json<GitHubInstallationBindingResponse>, ProviderSettingsError> {
    require_admin(&auth, "read")?;
    Ok(Json(installation_binding_response(
        provider_tokens
            .find_github_row(&GitHubScope::Global)
            .await?,
    )))
}

#[utoipa::path(
    put,
    path = "/api/v1/admin/settings/github/installation",
    tag = "provider-settings",
    request_body = GitHubInstallationBindingRequest,
    responses(
        (status = 200, description = "Global GitHub installation bound", body = GitHubInstallationBindingResponse),
        (status = 400, description = "Invalid App or installation", body = moatless_core::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn admin_put_global_github_installation(
    State(github_apps): State<Arc<GitHubAppRegistry>>,
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    State(adapter_connections): State<Arc<dyn AdapterConnectionDao>>,
    auth: AuthUser,
    Json(req): Json<GitHubInstallationBindingRequest>,
) -> Result<Json<GitHubInstallationBindingResponse>, ProviderSettingsError> {
    require_admin(&auth, "bind")?;
    Ok(Json(
        save_installation_binding(
            &github_apps,
            &provider_tokens,
            &adapter_connections,
            &auth.scope_context(),
            &GitHubScope::Global,
            req,
            &auth.user_id.to_string(),
        )
        .await?,
    ))
}

#[utoipa::path(
    delete,
    path = "/api/v1/admin/settings/github/installation",
    tag = "provider-settings",
    responses(
        (status = 200, description = "Global GitHub installation unbound", body = GitHubInstallationBindingResponse),
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn admin_delete_global_github_installation(
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    auth: AuthUser,
) -> Result<Json<GitHubInstallationBindingResponse>, ProviderSettingsError> {
    require_admin(&auth, "unbind")?;
    provider_tokens
        .delete_github_row(&GitHubScope::Global)
        .await?;
    Ok(Json(installation_binding_response(None)))
}

/// Save a GitHub Personal Access Token after validating it.
#[utoipa::path(
    put,
    path = "/api/v1/settings/github/pat",
    tag = "provider-settings",
    request_body = SavePatRequest,
    responses(
        (status = 200, description = "PAT saved", body = GitHubConfigResponse),
        (status = 400, description = "Invalid token", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn save_github_pat(
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    State(secrets): State<Arc<SecretService>>,
    auth: AuthUser,
    Json(req): Json<SavePatRequest>,
) -> Result<Json<GitHubConfigResponse>, ProviderSettingsError> {
    // Validate the token by calling GitHub API
    let login = github_client::validate_github_token(&req.token)
        .await
        .map_err(ProviderSettingsError::GitProvider)?;

    let mut metadata = serde_json::json!({ "login": login });

    // For GitHub App users, preserve the user access token so it can be restored later.
    if auth.provider.as_deref() == Some("github_app") {
        if let Ok(Some(existing)) = provider_tokens
            .find_provider_token_full(&auth.user_id.to_string(), "github")
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
                stash_github_oauth_token(
                    &secrets,
                    &auth.user_id.to_string(),
                    &existing.access_token,
                )
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
    secrets
        .put_secret(
            &auth.scope_context(),
            &auth.user_id.to_string(),
            PutSecretRequest {
                scope: Some(Scope::User),
                key: GITHUB_TOKEN.to_string(),
                kind: SecretKind::ProviderToken,
                provider: Some("github".to_string()),
                value: Some(req.token.clone()),
                external_ref: None,
            },
        )
        .await?;

    provider_tokens
        .upsert_token_with_auth_method(
            &auth.user_id.to_string(),
            "github",
            AUTH_METHOD_SECRET_REF,
            SECRET_REF_TOKEN_PLACEHOLDER,
            None,
            Some("pat"),
            Some(&metadata),
        )
        .await?;

    let is_github_app_user = auth.provider.as_deref() == Some("github_app");

    Ok(Json(GitHubConfigResponse {
        connected: true,
        login: Some(login),
        token_type: Some("pat".to_string()),
        has_pat_override: if is_github_app_user { Some(true) } else { None },
    }))
}

/// Disconnect GitHub by deleting the stored token.
#[utoipa::path(
    delete,
    path = "/api/v1/settings/github/config",
    tag = "provider-settings",
    responses(
        (status = 200, description = "GitHub disconnected", body = GitHubConfigResponse),
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn delete_github_config(
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    State(secrets): State<Arc<SecretService>>,
    auth: AuthUser,
) -> Result<Json<GitHubConfigResponse>, ProviderSettingsError> {
    provider_tokens
        .delete_token(&auth.user_id.to_string(), "github")
        .await?;

    for (key, kind) in [
        (GITHUB_TOKEN, SecretKind::ProviderToken),
        (GITHUB_OAUTH_STASH_KEY, SecretKind::Internal),
    ] {
        if let Err(e) = secrets
            .delete_user_secret(&auth.user_id.to_string(), key, Some(kind))
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

/// Remove a PAT override for a GitHub App user, restoring the App user token.
#[utoipa::path(
    delete,
    path = "/api/v1/settings/github/pat",
    tag = "provider-settings",
    responses(
        (status = 200, description = "PAT override removed", body = GitHubConfigResponse),
        (status = 400, description = "No PAT override to remove", body = moatless_core::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn delete_github_pat_override(
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    State(secrets): State<Arc<SecretService>>,
    auth: AuthUser,
) -> Result<Json<GitHubConfigResponse>, ProviderSettingsError> {
    if auth.provider.as_deref() != Some("github_app") {
        return Err(ProviderSettingsError::BadRequest(
            "PAT override removal is only available for GitHub App users".to_string(),
        ));
    }

    let token = provider_tokens
        .find_provider_token_full(&auth.user_id.to_string(), "github")
        .await?
        .ok_or_else(|| ProviderSettingsError::NotFound("No GitHub token found".to_string()))?;

    if token.token_type.as_deref() != Some("pat") {
        return Err(ProviderSettingsError::BadRequest(
            "No PAT override to remove".to_string(),
        ));
    }

    let oauth_token = read_github_oauth_stash(&secrets, &auth.user_id.to_string())
        .await?
        .ok_or_else(|| {
            ProviderSettingsError::BadRequest(
                "Cannot restore GitHub App token — original token not preserved".to_string(),
            )
        })?;

    let oauth_login = token
        .metadata
        .as_ref()
        .and_then(|m| m.get("oauth_login"))
        .and_then(|v| v.as_str())
        .map(std::string::ToString::to_string);

    // Restore the App user authorization branch used before the override.
    let restored_auth_method = token
        .metadata
        .as_ref()
        .and_then(|m| m.get("oauth_auth_method"))
        .and_then(|v| v.as_str())
        .unwrap_or("github_app")
        .to_string();

    let login = oauth_login.unwrap_or_else(|| auth.login.clone());
    let metadata = serde_json::json!({ "login": login });

    // Reset `auth_method` off `secret_ref` — the row carries a usable App user
    // token again, so the resolver must stop looking for a secret.
    provider_tokens
        .upsert_token_with_auth_method(
            &auth.user_id.to_string(),
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
        if let Err(e) = secrets
            .delete_user_secret(&auth.user_id.to_string(), key, Some(kind))
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
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn get_gitness_config(
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    auth: AuthUser,
) -> Result<Json<GitnessConfigResponse>, ProviderSettingsError> {
    let token = provider_tokens
        .find_provider_token_full(&auth.user_id.to_string(), "gitness")
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
        (status = 400, description = "Invalid token", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn save_gitness_pat(
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    auth: AuthUser,
    Json(req): Json<SaveGitnessPATRequest>,
) -> Result<Json<GitnessConfigResponse>, ProviderSettingsError> {
    // Validate the token by calling Gitness API
    let login = gitness_client::validate_gitness_token(&req.base_url, &req.token)
        .await
        .map_err(ProviderSettingsError::GitProvider)?;

    let metadata = serde_json::json!({
        "login": login,
        "base_url": req.base_url,
    });

    provider_tokens
        .upsert_token(
            &auth.user_id.to_string(),
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
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn delete_gitness_config(
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    auth: AuthUser,
) -> Result<Json<GitnessConfigResponse>, ProviderSettingsError> {
    provider_tokens
        .delete_token(&auth.user_id.to_string(), "gitness")
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
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn get_codex_config(
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    auth: AuthUser,
) -> Result<Json<CodexConfigResponse>, ProviderSettingsError> {
    let token = provider_tokens
        .find_provider_token_full(&auth.user_id.to_string(), "codex")
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
        (status = 400, description = "Invalid auth JSON", body = moatless_core::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn save_codex_auth(
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    State(credential_sync): State<Arc<CredentialSync>>,
    auth: AuthUser,
    Json(req): Json<SaveCodexAuthRequest>,
) -> Result<Json<CodexConfigResponse>, ProviderSettingsError> {
    let resp = save_codex_auth_for_scope(
        &provider_tokens,
        &credential_sync,
        CodexScope::user(auth.user_id.to_string()),
        req,
    )
    .await?;
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
        (status = 400, description = "Invalid auth JSON", body = moatless_core::error::ErrorBody),
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn admin_save_global_codex_auth(
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    State(credential_sync): State<Arc<CredentialSync>>,
    auth: AuthUser,
    Json(req): Json<SaveCodexAuthRequest>,
) -> Result<Json<CodexConfigResponse>, ProviderSettingsError> {
    if auth.role.as_deref() != Some("admin") {
        return Err(ProviderSettingsError::Forbidden(
            "Admin role required to set the global Codex blob".to_string(),
        ));
    }
    let resp =
        save_codex_auth_for_scope(&provider_tokens, &credential_sync, CodexScope::Global, req)
            .await?;
    Ok(Json(resp))
}

/// Delete the instance-wide (global) Codex blob. Admin only.
#[utoipa::path(
    delete,
    path = "/api/v1/admin/settings/codex/config",
    tag = "provider-settings",
    responses(
        (status = 200, description = "Global codex disconnected", body = CodexConfigResponse),
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody),
        (status = 403, description = "Admin only", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn admin_delete_global_codex_config(
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    auth: AuthUser,
) -> Result<Json<CodexConfigResponse>, ProviderSettingsError> {
    if auth.role.as_deref() != Some("admin") {
        return Err(ProviderSettingsError::Forbidden(
            "Admin role required to delete the global Codex blob".to_string(),
        ));
    }
    provider_tokens
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
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn delete_codex_config(
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    auth: AuthUser,
) -> Result<Json<CodexConfigResponse>, ProviderSettingsError> {
    provider_tokens
        .delete_codex_blob(&CodexScope::user(auth.user_id.to_string()))
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
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody),
        (status = 502, description = "Codex OAuth provider error", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn start_codex_device_login(
    State(codex_oauth_config): State<Arc<CodexOAuthConfig>>,
    _auth: AuthUser,
) -> Result<Json<CodexDeviceCodeStartResponse>, ProviderSettingsError> {
    let client = reqwest::Client::new();
    let started = codex_oauth::request_device_code(
        &client,
        &codex_oauth_config.issuer,
        &codex_oauth_config.client_id,
    )
    .await
    .map_err(ProviderSettingsError::CodexOAuth)?;

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
        (status = 401, description = "Not authenticated", body = moatless_core::error::ErrorBody),
        (status = 502, description = "Codex OAuth provider error", body = moatless_core::error::ErrorBody)
    )
)]
pub async fn poll_codex_device_login(
    State(codex_oauth_config): State<Arc<CodexOAuthConfig>>,
    State(provider_tokens): State<Arc<dyn ProviderTokenDao>>,
    State(credential_sync): State<Arc<CredentialSync>>,
    auth: AuthUser,
    Json(req): Json<CodexDeviceCodePollRequest>,
) -> Result<Json<CodexDeviceCodePollResponse>, ProviderSettingsError> {
    let client = reqwest::Client::new();
    let outcome = codex_oauth::poll_once(
        &client,
        &codex_oauth_config.issuer,
        &codex_oauth_config.client_id,
        &req.device_auth_id,
        &req.user_code,
    )
    .await
    .map_err(ProviderSettingsError::CodexOAuth)?;

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
            provider_tokens
                .upsert_token(
                    &auth.user_id.to_string(),
                    "codex",
                    &serialized,
                    None,
                    Some("auth_json"),
                    Some(&metadata),
                )
                .await?;

            // Push the already-persisted auth.json to the user's running
            // sandboxes so they pick up the new tokens without a restart.
            // Broadcast directly rather than through `store`: the row was just
            // written above, so `store` would read it back, dedup on the
            // identical value, and never fan out. The echo loop terminates one
            // hop in — a sandbox echoing the push back arrives as
            // `store(.., SandboxRotation)`, which dedups and does not
            // re-broadcast.
            credential_sync
                .broadcast_codex_rotation(&CodexScope::user(auth.user_id.to_string()), &serialized)
                .await;

            Ok(Json(CodexDeviceCodePollResponse::Success {
                connected: true,
                email: id_token_claims.email,
                plan_type: id_token_claims.chatgpt_plan_type,
            }))
        }
    }
}
