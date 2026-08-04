//! Adapter registry construction from application configuration.

use std::sync::Arc;

use crate::compose::Ports;

use super::Adapters;

impl Adapters {
    /// Build adapters from application config and the composed ports.
    ///
    /// Returns `None` when no providers are configured.
    ///
    /// `ports.github_apps` is the shared App registry, so the `GithubPrAdapter`
    /// and `AppState` hit the same token cache. The `GithubPrAdapter` resolves
    /// runtime GitHub tokens through the canonical resolver, which needs the
    /// registry to mint installation tokens for bot users (`auth_method =
    /// 'github_app_installation'`). Slack, Telegram and Linear read their own
    /// per-workspace rows straight off `ports.pool`.
    pub async fn from_config(config: &crate::config::Config, ports: &Ports) -> Option<Self> {
        let mut providers = Self::new();

        let db = ports.pool.clone();
        let provider_tokens = ports.provider_tokens.clone();
        let provider_token_auth = ports.provider_token_auth.clone();
        let github_apps = ports.github_apps.clone();
        let github_app_service = ports.github_app_service.clone();
        // Needed so the GitHub adapter can resolve `secret_ref` provider tokens
        // (validated PATs live in `secrets`, not `provider_tokens`).
        let secrets = ports.secrets.clone();

        let runtime_config = Arc::new(config.clone());
        let slack = crate::adapters::slack::adapter::SlackAdapter::from_db(
            runtime_config.clone(),
            db.clone(),
        );
        providers.register_slack(Arc::new(slack));
        tracing::info!("Slack adapter registered");

        // Webhook adapter is always available (config is per subscription).
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

        // GitHub PR adapter is always available; config is per subscription.
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
