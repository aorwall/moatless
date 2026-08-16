//! Installation-wide Adapter and GitHub App administration.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use moatless_backend_core::scope::{Scope, ScopeContext};

use crate::adapter_apps::dao::{AdapterAppDao, GitHubAppAuditEntry, GitHubAppRecord, NewGitHubApp};
use crate::adapter_apps::{
    AdapterAppInput, AdapterAppSecretInput, DEFAULT_APP_KEY, StoredAdapterApp,
};
use crate::adapters::dao::AdapterConnectionDao;
use crate::adapters::models::AdapterConnection;
use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::service::{GitHubAppService, ProvedApp};
use crate::auth::github_scope::GitHubScope;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::{
    APP_INSTALLATION_TOKEN_PLACEHOLDER, AUTH_METHOD_APP_INSTALLATION,
};
use crate::auth::users::UserDao;
use crate::secret::service::SecretCipher;

const GITHUB_ADAPTER_KIND: &str = "github";

#[derive(Clone, Debug)]
pub enum AdapterAppSave {
    Slack {
        bot_token: String,
        signing_secret: String,
        bot_user_id: Option<String>,
    },
    Telegram {
        bot_token: String,
        mode: Option<String>,
    },
    Linear {
        webhook_secret: Option<String>,
        oauth_client_id: Option<String>,
        oauth_client_secret: Option<String>,
        oauth_callback_url: Option<String>,
        api_base_url: Option<String>,
    },
}

#[derive(Clone, Debug)]
pub struct GitHubAppRegistration {
    pub github_app_key: String,
    pub app_id: u64,
    pub name: String,
    pub slug: String,
    pub bot_login: String,
    pub key_fingerprint: String,
}

#[derive(Clone, Debug)]
pub struct GitHubAppRegistrationInput {
    pub github_app_key: String,
    pub app_id: u64,
    pub private_key_pem: String,
    pub default_installation_id: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct GitHubInstallation {
    pub installation_id: u64,
    pub account_login: String,
    pub account_type: Option<String>,
}

#[derive(Clone, Debug)]
pub struct GitHubAppView {
    pub github_app_key: String,
    pub name: String,
    pub install_url: String,
    pub installations: Vec<GitHubInstallation>,
    pub deployment_configured: bool,
    pub key_fingerprint: Option<String>,
    pub bot_login: Option<String>,
    pub bot_user_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GitHubInstallationBinding {
    pub configured: bool,
    pub github_app_key: Option<String>,
    pub installation_id: Option<u64>,
    pub account_login: Option<String>,
    pub account_type: Option<String>,
}

#[derive(Clone, Debug)]
pub struct RemovedGitHubApp {
    pub github_app_key: String,
    pub orphaned_bot_users: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AppAdministrationError {
    #[error("Not found: {0}")]
    NotFound(String),
    #[error("Bad request: {0}")]
    BadRequest(String),
    #[error("Conflict: {0}")]
    Conflict(String),
    #[error("Forbidden: {0}")]
    Forbidden(String),
    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("Provider error: {0}")]
    GitHubApp(#[source] crate::auth::error::AuthError),
}

impl From<crate::auth::error::AuthError> for AppAdministrationError {
    fn from(error: crate::auth::error::AuthError) -> Self {
        use crate::auth::error::AuthError;
        match error {
            AuthError::BadRequest(message) => Self::BadRequest(message),
            AuthError::NotFound(message) => Self::NotFound(message),
            AuthError::Conflict(message) => Self::Conflict(message),
            AuthError::Forbidden => Self::Forbidden("Access denied".to_string()),
            other => Self::GitHubApp(other),
        }
    }
}

#[async_trait]
pub trait GitHubAppProver: Send + Sync {
    async fn prove(
        &self,
        app_key: &str,
        app_id: u64,
        private_key_pem: &str,
        default_installation_id: Option<u64>,
    ) -> Result<ProvedApp, AppAdministrationError>;
}

pub struct ProviderGitHubAppProver;

#[async_trait]
impl GitHubAppProver for ProviderGitHubAppProver {
    async fn prove(
        &self,
        app_key: &str,
        app_id: u64,
        private_key_pem: &str,
        default_installation_id: Option<u64>,
    ) -> Result<ProvedApp, AppAdministrationError> {
        let entry = moatless_core::config::GitHubAppConfig {
            key: app_key.to_string(),
            app_id,
            private_key: private_key_pem.to_string(),
            client_id: None,
            client_secret: None,
            default_installation_id,
        };
        let service = GitHubAppService::from_app_config(&entry).ok_or_else(|| {
            AppAdministrationError::BadRequest(
                "The signing key could not be read as a private key. Nothing was stored."
                    .to_string(),
            )
        })?;
        Ok(service.prove().await?)
    }
}

pub struct AppAdministration {
    github_apps: Arc<GitHubAppRegistry>,
    provider_tokens: Arc<dyn ProviderTokenDao>,
    adapter_connections: Arc<dyn AdapterConnectionDao>,
    users: Arc<dyn UserDao>,
    prover: Arc<dyn GitHubAppProver>,
}

type AdministrationStore = (Arc<dyn AdapterAppDao>, Arc<dyn SecretCipher>);

impl AppAdministration {
    pub fn new(
        github_apps: Arc<GitHubAppRegistry>,
        provider_tokens: Arc<dyn ProviderTokenDao>,
        adapter_connections: Arc<dyn AdapterConnectionDao>,
        users: Arc<dyn UserDao>,
        prover: Arc<dyn GitHubAppProver>,
    ) -> Self {
        Self {
            github_apps,
            provider_tokens,
            adapter_connections,
            users,
            prover,
        }
    }

    pub async fn list_adapter_apps(&self) -> Result<Vec<StoredAdapterApp>, AppAdministrationError> {
        Ok(self.store()?.list_adapter_apps().await?)
    }

    pub async fn save_adapter_app(
        &self,
        adapter_kind: &str,
        app_key: &str,
        input: AdapterAppSave,
        actor_id: &str,
    ) -> Result<StoredAdapterApp, AppAdministrationError> {
        if app_key != DEFAULT_APP_KEY {
            return Err(AppAdministrationError::BadRequest(format!(
                "Only the '{DEFAULT_APP_KEY}' Adapter app key is supported for this Adapter"
            )));
        }
        if adapter_kind == "github" {
            return Err(AppAdministrationError::BadRequest(
                "Use the GitHub App admin endpoints for GitHub App registration".to_string(),
            ));
        }
        let app = adapter_app_input(adapter_kind, app_key, input)?;
        let (store, cipher) = self.administration()?;
        Ok(store
            .upsert_adapter_app(app, cipher.as_ref(), actor_id)
            .await?)
    }

    pub async fn delete_adapter_app(
        &self,
        adapter_kind: &str,
        app_key: &str,
    ) -> Result<(), AppAdministrationError> {
        if adapter_kind == "github" {
            return Err(AppAdministrationError::BadRequest(
                "Use the GitHub App admin endpoints to remove GitHub Apps".to_string(),
            ));
        }
        self.store()?
            .delete_adapter_app(adapter_kind, app_key)
            .await?;
        Ok(())
    }

    pub async fn list_github_apps(&self) -> Result<Vec<GitHubAppView>, AppAdministrationError> {
        let entries = self.github_apps.entries().await?;
        let stored = match self.github_apps.store() {
            Some(store) => store.list_github_apps().await?,
            None => Vec::new(),
        };
        let stored_apps: HashMap<String, GitHubAppRecord> = stored
            .into_iter()
            .map(|record| (record.app_key.clone(), record))
            .collect();
        let bot_users = self.users.list_bot_users_by_app_key().await?;
        let mut apps = Vec::new();
        for (github_app_key, app) in entries {
            let (name, install_url) = app.app_listing().await?;
            let installations = app
                .list_installations()
                .await?
                .into_iter()
                .map(|installation| GitHubInstallation {
                    installation_id: installation.id,
                    account_login: installation.account.login,
                    account_type: installation.account.account_type,
                })
                .collect();
            let stored = stored_apps.get(&github_app_key);
            apps.push(GitHubAppView {
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
        Ok(apps)
    }

    pub async fn register_github_app(
        &self,
        input: GitHubAppRegistrationInput,
        actor_id: &str,
    ) -> Result<GitHubAppRegistration, AppAdministrationError> {
        let (store, cipher) = self.administration()?;
        if let Err(error) = validate_app_key(&input.github_app_key) {
            self.record_refusal(
                store.as_ref(),
                &input.github_app_key,
                actor_id,
                "register",
                &error,
            )
            .await;
            return Err(error);
        }
        let proved = match self
            .prover
            .prove(
                &input.github_app_key,
                input.app_id,
                &input.private_key_pem,
                input.default_installation_id,
            )
            .await
        {
            Ok(proved) => proved,
            Err(error) => {
                self.record_refusal(
                    store.as_ref(),
                    &input.github_app_key,
                    actor_id,
                    "register",
                    &error,
                )
                .await;
                return Err(error);
            }
        };
        let record = GitHubAppRecord::seal(
            NewGitHubApp {
                app_key: input.github_app_key.clone(),
                app_id: proved.app_id,
                name: proved.name.clone(),
                slug: proved.slug.clone(),
                bot_login: proved.bot_login.clone(),
                private_key_pem: input.private_key_pem,
                default_installation_id: input.default_installation_id,
            },
            cipher.as_ref(),
            None,
        )?;
        if let Err(error) = store.insert_github_app(&record, actor_id).await {
            let error = AppAdministrationError::from(error);
            self.record_refusal(
                store.as_ref(),
                &input.github_app_key,
                actor_id,
                "register",
                &error,
            )
            .await;
            return Err(error);
        }
        self.record_audit(
            store.as_ref(),
            Some(&record.id),
            &input.github_app_key,
            actor_id,
            "register",
            "ok",
            None,
        )
        .await;
        Ok(registration(
            input.github_app_key,
            proved,
            record.key_fingerprint,
        ))
    }

    pub async fn rotate_github_app_key(
        &self,
        github_app_key: &str,
        private_key_pem: String,
        actor_id: &str,
    ) -> Result<GitHubAppRegistration, AppAdministrationError> {
        let (store, cipher) = self.administration()?;
        let existing = store
            .find_github_app_by_key(github_app_key)
            .await?
            .ok_or_else(|| not_registered(github_app_key))?;
        let proved = match self
            .prover
            .prove(
                github_app_key,
                existing.app_id,
                &private_key_pem,
                existing.default_installation_id,
            )
            .await
        {
            Ok(proved) => proved,
            Err(error) => {
                self.record_refusal(store.as_ref(), github_app_key, actor_id, "rotate", &error)
                    .await;
                return Err(error);
            }
        };
        let record = GitHubAppRecord::seal(
            NewGitHubApp {
                app_key: github_app_key.to_string(),
                app_id: proved.app_id,
                name: proved.name.clone(),
                slug: proved.slug.clone(),
                bot_login: proved.bot_login.clone(),
                private_key_pem,
                default_installation_id: existing.default_installation_id,
            },
            cipher.as_ref(),
            Some(existing.id),
        )?;
        if !store.replace_github_app(&record, actor_id).await? {
            return Err(not_registered(github_app_key));
        }
        self.github_apps.forget(github_app_key).await;
        self.record_audit(
            store.as_ref(),
            Some(&record.id),
            github_app_key,
            actor_id,
            "rotate",
            "ok",
            None,
        )
        .await;
        Ok(registration(
            github_app_key.to_string(),
            proved,
            record.key_fingerprint,
        ))
    }

    pub async fn remove_github_app(
        &self,
        github_app_key: &str,
        actor_id: &str,
    ) -> Result<RemovedGitHubApp, AppAdministrationError> {
        let store = self.store()?;
        let existing = store
            .find_github_app_by_key(github_app_key)
            .await?
            .ok_or_else(|| not_registered(github_app_key))?;
        let orphaned_bot_users: Vec<String> = self
            .users
            .list_bot_users_by_app_key()
            .await?
            .remove(github_app_key)
            .into_iter()
            .collect();
        if !store.delete_github_app(github_app_key).await? {
            return Err(not_registered(github_app_key));
        }
        self.github_apps.forget(github_app_key).await;
        let detail = (!orphaned_bot_users.is_empty()).then(|| {
            format!(
                "{} bot user(s) left without a key",
                orphaned_bot_users.len()
            )
        });
        self.record_audit(
            store.as_ref(),
            Some(&existing.id),
            github_app_key,
            actor_id,
            "remove",
            "ok",
            detail.as_deref(),
        )
        .await;
        Ok(RemovedGitHubApp {
            github_app_key: github_app_key.to_string(),
            orphaned_bot_users,
        })
    }

    pub async fn global_github_installation(
        &self,
    ) -> Result<GitHubInstallationBinding, AppAdministrationError> {
        Ok(installation_binding(
            self.provider_tokens
                .find_github_row(&GitHubScope::Global)
                .await?,
        ))
    }

    pub async fn bind_global_github_installation(
        &self,
        github_app_key: String,
        installation_id: u64,
        actor_id: &str,
        scope_context: &ScopeContext,
    ) -> Result<GitHubInstallationBinding, AppAdministrationError> {
        let app = self.github_apps.require(&github_app_key).await?;
        let installation = app.get_installation(installation_id).await?;
        let metadata = serde_json::json!({
            "installation_id": installation.id,
            "account_login": installation.account.login,
            "account_type": installation.account.account_type,
        });
        self.provider_tokens
            .upsert_github_row(
                &GitHubScope::Global,
                AUTH_METHOD_APP_INSTALLATION,
                APP_INSTALLATION_TOKEN_PLACEHOLDER,
                None,
                Some(&metadata),
                Some(&github_app_key),
                Some(&installation.account.login),
            )
            .await?;
        self.mirror_github_connection(
            scope_context,
            &installation.account.login,
            &github_app_key,
            installation.id,
            actor_id,
        )
        .await?;
        self.global_github_installation().await
    }

    pub async fn unbind_global_github_installation(
        &self,
    ) -> Result<GitHubInstallationBinding, AppAdministrationError> {
        self.provider_tokens
            .delete_github_row(&GitHubScope::Global)
            .await?;
        Ok(installation_binding(None))
    }

    fn store(&self) -> Result<Arc<dyn AdapterAppDao>, AppAdministrationError> {
        self.github_apps.store().map(Arc::clone).ok_or_else(|| {
            AppAdministrationError::BadRequest(
                "This installation cannot store Adapter Apps: it has no database configured."
                    .to_string(),
            )
        })
    }

    fn administration(&self) -> Result<AdministrationStore, AppAdministrationError> {
        let (Some(store), Some(cipher)) = (self.github_apps.store(), self.github_apps.cipher())
        else {
            return Err(AppAdministrationError::BadRequest(
                "This installation cannot store GitHub Apps: it has no encryption configured. Set SECRETS_ENCRYPTION_KEY and try again."
                    .to_string(),
            ));
        };
        Ok((Arc::clone(store), Arc::clone(cipher)))
    }

    async fn mirror_github_connection(
        &self,
        context: &ScopeContext,
        account_login: &str,
        github_app_key: &str,
        installation_id: u64,
        actor_id: &str,
    ) -> Result<(), AppAdministrationError> {
        let existing = self
            .adapter_connections
            .find_by_external_account(GITHUB_ADAPTER_KIND, account_login)
            .await?;
        let now = chrono::Utc::now();
        let mut transport_config = existing
            .as_ref()
            .map(|connection| connection.transport_config.clone())
            .filter(serde_json::Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        if let Some(fields) = transport_config.as_object_mut() {
            fields.insert("github_app_key".to_string(), github_app_key.into());
            fields.insert("installation_id".to_string(), installation_id.into());
        }
        let row = AdapterConnection {
            id: existing.as_ref().map_or_else(
                || format!("pc_{}", uuid::Uuid::new_v4()),
                |row| row.id.clone(),
            ),
            adapter_kind: GITHUB_ADAPTER_KIND.to_string(),
            connection_kind: existing.as_ref().map_or_else(
                || "org_webhook".to_string(),
                |row| row.connection_kind.clone(),
            ),
            scope_value: account_login.to_string(),
            credential_ref: existing.as_ref().and_then(|row| row.credential_ref.clone()),
            webhook_secret: existing.as_ref().and_then(|row| row.webhook_secret.clone()),
            transport_config,
            created_by: existing
                .as_ref()
                .map_or_else(|| actor_id.to_string(), |row| row.created_by.clone()),
            scope: Scope::Global,
            owner_user_id: None,
            created_at: existing.as_ref().map_or(now, |row| row.created_at),
            updated_at: now,
        };
        self.adapter_connections.upsert(&row, context).await?;
        Ok(())
    }

    async fn record_refusal(
        &self,
        store: &dyn AdapterAppDao,
        app_key: &str,
        actor_id: &str,
        action: &str,
        error: &AppAdministrationError,
    ) {
        self.record_audit(
            store,
            None,
            app_key,
            actor_id,
            action,
            "refused",
            Some(&error.to_string()),
        )
        .await;
    }

    #[expect(clippy::too_many_arguments, reason = "one complete audit record")]
    async fn record_audit(
        &self,
        store: &dyn AdapterAppDao,
        github_app_id: Option<&str>,
        app_key: &str,
        actor_id: &str,
        action: &str,
        result: &str,
        detail: Option<&str>,
    ) {
        if let Err(error) = store
            .record_github_app_audit(GitHubAppAuditEntry {
                github_app_id,
                app_key,
                actor_id,
                action,
                result,
                detail,
            })
            .await
        {
            tracing::warn!(app_key, action, %error, "Could not record GitHub App audit event");
        }
    }
}

fn adapter_app_input(
    adapter_kind: &str,
    app_key: &str,
    input: AdapterAppSave,
) -> Result<AdapterAppInput, AppAdministrationError> {
    let mismatch = || {
        AppAdministrationError::BadRequest(
            "Request body adapterKind must match the path".to_string(),
        )
    };
    match input {
        AdapterAppSave::Slack {
            bot_token,
            signing_secret,
            bot_user_id,
        } => {
            if adapter_kind != "slack" {
                return Err(mismatch());
            }
            Ok(AdapterAppInput {
                adapter_kind: "slack",
                app_key: app_key.to_string(),
                display_name: "Slack".to_string(),
                config_json: serde_json::json!({ "bot_user_id": bot_user_id }),
                secrets: vec![
                    AdapterAppSecretInput {
                        name: "bot_token",
                        plaintext: bot_token,
                    },
                    AdapterAppSecretInput {
                        name: "signing_secret",
                        plaintext: signing_secret,
                    },
                ],
            })
        }
        AdapterAppSave::Telegram { bot_token, mode } => {
            if adapter_kind != "telegram" {
                return Err(mismatch());
            }
            let mode = mode.unwrap_or_else(|| "webhook".to_string());
            if mode != "webhook" && mode != "polling" {
                return Err(AppAdministrationError::BadRequest(
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
                    plaintext: bot_token,
                }],
            })
        }
        AdapterAppSave::Linear {
            webhook_secret,
            oauth_client_id,
            oauth_client_secret,
            oauth_callback_url,
            api_base_url,
        } => {
            if adapter_kind != "linear" {
                return Err(mismatch());
            }
            let mut secrets = Vec::new();
            for (name, value) in [
                ("webhook_secret", webhook_secret),
                ("oauth_client_id", oauth_client_id),
                ("oauth_client_secret", oauth_client_secret),
            ] {
                if let Some(plaintext) = value {
                    secrets.push(AdapterAppSecretInput { name, plaintext });
                }
            }
            Ok(AdapterAppInput {
                adapter_kind: "linear",
                app_key: app_key.to_string(),
                display_name: "Linear".to_string(),
                config_json: serde_json::json!({
                    "oauth_callback_url": oauth_callback_url,
                    "api_base_url": api_base_url,
                }),
                secrets,
            })
        }
    }
}

fn validate_app_key(app_key: &str) -> Result<(), AppAdministrationError> {
    if app_key == GitHubAppService::DEFAULT_KEY {
        return Err(AppAdministrationError::BadRequest(format!(
            "'{}' names the App this deployment signs in with and cannot be registered",
            GitHubAppService::DEFAULT_KEY
        )));
    }
    let valid = !app_key.is_empty()
        && app_key.len() <= 64
        && app_key.starts_with(|character: char| character.is_ascii_alphanumeric())
        && app_key.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        });
    if !valid {
        return Err(AppAdministrationError::BadRequest(format!(
            "'{app_key}' is not a usable App name: use letters, digits, '-' and '_', starting with a letter or digit, at most 64 characters"
        )));
    }
    Ok(())
}

fn not_registered(app_key: &str) -> AppAdministrationError {
    AppAdministrationError::NotFound(format!("No GitHub App is registered as '{app_key}'"))
}

fn registration(
    github_app_key: String,
    proved: ProvedApp,
    key_fingerprint: String,
) -> GitHubAppRegistration {
    GitHubAppRegistration {
        github_app_key,
        app_id: proved.app_id,
        name: proved.name,
        slug: proved.slug,
        bot_login: proved.bot_login,
        key_fingerprint,
    }
}

fn installation_binding(
    row: Option<crate::auth::models::ProviderToken>,
) -> GitHubInstallationBinding {
    let Some(row) = row else {
        return GitHubInstallationBinding {
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
    let account_login = metadata_string(row.metadata.as_ref(), "account_login");
    let account_type = metadata_string(row.metadata.as_ref(), "account_type");
    let github_app_key = row.provider_user_id;
    GitHubInstallationBinding {
        configured: row.auth_method.as_deref() == Some(AUTH_METHOD_APP_INSTALLATION)
            && github_app_key.is_some()
            && installation_id.is_some()
            && account_login.is_some(),
        github_app_key,
        installation_id,
        account_login,
        account_type,
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
    use super::*;
    use crate::auth::test_support::StubProviderTokens;
    use crate::auth::users::MockUserDao;
    use crate::secret::service::LocalEnvelopeSecretCipher;
    use moatless_identity::adapter_apps::dao::test_support::InMemoryAdapterAppDao;

    struct RefusingProver;

    #[async_trait]
    impl GitHubAppProver for RefusingProver {
        async fn prove(
            &self,
            _app_key: &str,
            _app_id: u64,
            _private_key_pem: &str,
            _default_installation_id: Option<u64>,
        ) -> Result<ProvedApp, AppAdministrationError> {
            Err(AppAdministrationError::BadRequest(
                "signing-key proof failed".to_string(),
            ))
        }
    }

    struct UnusedConnections;

    #[async_trait]
    impl AdapterConnectionDao for UnusedConnections {
        async fn find_by_id(
            &self,
            _id: &str,
            _ctx: &ScopeContext,
        ) -> Result<Option<AdapterConnection>, sqlx::Error> {
            unreachable!("registration does not read Adapter connections")
        }

        async fn find_by_external_account(
            &self,
            _adapter_kind: &str,
            _external_account_id: &str,
        ) -> Result<Option<AdapterConnection>, sqlx::Error> {
            unreachable!("registration does not read Adapter connections")
        }

        async fn find_by_scope_value(
            &self,
            _adapter_kind: &str,
            _scope_value: &str,
        ) -> Result<Option<AdapterConnection>, sqlx::Error> {
            unreachable!("registration does not read Adapter connections")
        }

        async fn list_by_provider(
            &self,
            _adapter_kind: &str,
        ) -> Result<Vec<AdapterConnection>, sqlx::Error> {
            unreachable!("registration does not list Adapter connections")
        }

        async fn list(
            &self,
            _ctx: &ScopeContext,
            _adapter_kind: Option<&str>,
        ) -> Result<Vec<AdapterConnection>, sqlx::Error> {
            unreachable!("registration does not list Adapter connections")
        }

        async fn upsert(
            &self,
            _row: &AdapterConnection,
            _ctx: &ScopeContext,
        ) -> Result<AdapterConnection, sqlx::Error> {
            unreachable!("registration does not write Adapter connections")
        }

        async fn delete(&self, _id: &str, _ctx: &ScopeContext) -> Result<u64, sqlx::Error> {
            unreachable!("registration does not delete Adapter connections")
        }
    }

    #[tokio::test]
    async fn failed_signing_key_proof_records_refusal_before_any_mutation() {
        let store = Arc::new(InMemoryAdapterAppDao::default());
        let store_port: Arc<dyn AdapterAppDao> = store.clone();
        let cipher =
            LocalEnvelopeSecretCipher::from_config(&moatless_core::config::SecretsConfig {
                encryption_key: Some("0123456789abcdef0123456789abcdef".to_string()),
                kms_key_id: None,
            })
            .expect("test cipher");
        let registry = Arc::new(GitHubAppRegistry::new(None, Some(store_port), Some(cipher)));
        let administration = AppAdministration::new(
            registry,
            Arc::new(StubProviderTokens::default()),
            Arc::new(UnusedConnections),
            Arc::new(MockUserDao::new()),
            Arc::new(RefusingProver),
        );

        let result = administration
            .register_github_app(
                GitHubAppRegistrationInput {
                    github_app_key: "reviewer".to_string(),
                    app_id: 42,
                    private_key_pem: "invalid".to_string(),
                    default_installation_id: None,
                },
                "admin",
            )
            .await;

        assert!(matches!(result, Err(AppAdministrationError::BadRequest(_))));
        assert_eq!(store.github_app_count(), 0);
        assert_eq!(
            store.audit_entries(),
            vec![(
                "reviewer".to_string(),
                "register".to_string(),
                "refused".to_string(),
            )]
        );
    }

    #[tokio::test]
    async fn removal_reports_bot_users_orphaned_by_the_app() {
        let store = Arc::new(InMemoryAdapterAppDao::default());
        let cipher =
            LocalEnvelopeSecretCipher::from_config(&moatless_core::config::SecretsConfig {
                encryption_key: Some("0123456789abcdef0123456789abcdef".to_string()),
                kms_key_id: None,
            })
            .expect("test cipher");
        let record = GitHubAppRecord::seal(
            NewGitHubApp {
                app_key: "reviewer".to_string(),
                app_id: 42,
                name: "Reviewer".to_string(),
                slug: "reviewer".to_string(),
                bot_login: "reviewer[bot]".to_string(),
                private_key_pem: "test-key".to_string(),
                default_installation_id: None,
            },
            cipher.as_ref(),
            None,
        )
        .expect("record should seal");
        store.put(record);
        let store_port: Arc<dyn AdapterAppDao> = store.clone();
        let registry = Arc::new(GitHubAppRegistry::new(None, Some(store_port), Some(cipher)));
        let mut users = MockUserDao::new();
        users.expect_list_bot_users_by_app_key().return_once(|| {
            Ok(HashMap::from([(
                "reviewer".to_string(),
                "bot-user-id".to_string(),
            )]))
        });
        let administration = AppAdministration::new(
            registry,
            Arc::new(StubProviderTokens::default()),
            Arc::new(UnusedConnections),
            Arc::new(users),
            Arc::new(RefusingProver),
        );

        let removed = administration
            .remove_github_app("reviewer", "admin")
            .await
            .expect("registered app should be removed");

        assert_eq!(removed.orphaned_bot_users, vec!["bot-user-id"]);
        assert_eq!(store.github_app_count(), 0);
    }
}
