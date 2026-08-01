//! Adapter registry construction from application configuration.

use std::sync::Arc;

use crate::auth::github::registry::GitHubAppRegistry;
use crate::db::DbPool;

use super::Adapters;

impl Adapters {
    /// Build adapters from application config.
    ///
    /// Returns `None` when no providers are configured.
    ///
    /// `github_apps` is the shared App registry — passed in so the
    /// `GithubPrAdapter` and `AppState` hit the same token cache. The
    /// `GithubPrAdapter` resolves runtime GitHub tokens through the canonical
    /// resolver, which needs the registry to mint installation tokens for bot
    /// users (`auth_method = 'github_app_installation'`).
    pub async fn from_config(
        config: &crate::config::Config,
        db: DbPool,
        github_apps: Arc<GitHubAppRegistry>,
    ) -> Option<Self> {
        use crate::auth::provider_token_dao::{PgProviderTokenDao, ProviderTokenDao};
        use crate::auth::provider_tokens::{PgProviderTokenAuthDao, ProviderTokenAuthDao};
        use crate::secret::audit::{PgSecretAuditDao, SecretAuditDao};
        use crate::secret::dao::{PgSecretDao, SecretDao};
        use crate::secret::service::{LocalEnvelopeSecretCipher, SecretService};

        let mut providers = Self::new();

        let provider_tokens: Arc<dyn ProviderTokenDao> = Arc::new(PgProviderTokenDao(db.clone()));
        let provider_token_auth: Arc<dyn ProviderTokenAuthDao> =
            Arc::new(PgProviderTokenAuthDao(db.clone()));
        let github_app_service = github_apps.default_service();

        // Needed so the GitHub adapter can resolve `secret_ref` provider tokens
        // (validated PATs live in `secrets`, not `provider_tokens`).
        let secret_records: Arc<dyn SecretDao> = Arc::new(PgSecretDao(db.clone()));
        let secret_audits: Arc<dyn SecretAuditDao> = Arc::new(PgSecretAuditDao(db.clone()));
        let secrets = Arc::new(SecretService::new(
            secret_records,
            secret_audits,
            LocalEnvelopeSecretCipher::from_config(&config.secrets),
        ));

        let runtime_config = Arc::new(config.clone());
        let slack = crate::adapters::slack::adapter::SlackAdapter::from_db(
            runtime_config.clone(),
            db.clone(),
        );
        providers.register_slack(Arc::new(slack));
        tracing::info!("Slack adapter registered");

        // Webhook adapter is always available (config is per trigger source).
        providers.register_webhook(Arc::new(
            crate::adapters::webhook::adapter::WebhookAdapter::new(),
        ));
        tracing::info!("Webhook adapter configured");

        let telegram = crate::adapters::telegram::adapter::TelegramAdapter::from_db(
            runtime_config,
            db.clone(),
        );
        providers.register_telegram(Arc::new(telegram));
        tracing::info!("Telegram adapter registered");

        // GitHub PR adapter is always available; config is per trigger source.
        {
            let github_pr = crate::adapters::github_pr::adapter::GithubPrAdapter::new(
                config.github_pr.webhook_secret.clone(),
                config.github_pr.bot_token.clone(),
                config.auth.frontend_url.clone(),
                provider_tokens.clone(),
                Some(provider_token_auth.clone()),
                github_app_service.clone(),
                github_apps.clone(),
            )
            .with_secrets(secrets.clone());
            providers.register_github_pr(Arc::new(github_pr));
            tracing::info!("GitHub PR adapter configured");
        }

        // Linear adapter — workspace tokens come from the OAuth install
        // flow (provider_tokens, team_id=<linear org uuid>). OAuth client
        // credentials are needed to refresh expired access tokens.
        {
            let linear = crate::adapters::linear::adapter::LinearAdapter::from_db(
                Arc::new(config.clone()),
                db,
            );
            providers.register_linear(Arc::new(linear));
            tracing::info!("Linear adapter configured");
        }

        Some(providers)
    }
}
