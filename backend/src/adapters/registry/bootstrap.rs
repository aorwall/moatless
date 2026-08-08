//! Adapter registry construction from application configuration.

use std::sync::Arc;

use moatless_core::db::DbPool;

use crate::auth::github::registry::GitHubAppRegistry;
use crate::auth::github::service::GitHubAppService;
use crate::auth::provider_token_dao::ProviderTokenDao;
use crate::auth::provider_tokens::ProviderTokenAuthDao;
use crate::secret::service::SecretService;

use super::{Adapters, ProviderHandles};

impl Adapters {
    /// Build adapters from application config and the composed ports, returning
    /// the trait-object registry and the concrete per-provider handles the root
    /// hands to each provider's own handlers.
    ///
    /// Every provider (Slack, Webhook, Telegram, GitHub PR, Linear) is registered
    /// unconditionally — their per-workspace config is resolved per subscription,
    /// not at registration — so the registry is always populated, and every
    /// concrete-handle field of the returned `ProviderHandles` is `Some`.
    ///
    /// `github_apps` is the shared App registry, so the `GithubPrAdapter`
    /// and `AppState` hit the same token cache. The `GithubPrAdapter` resolves
    /// runtime GitHub tokens through the canonical resolver, which needs the
    /// registry to mint installation tokens for bot users (`auth_method =
    /// 'github_app_installation'`). Slack, Telegram and Linear read their own
    /// per-workspace rows straight off `pool`.
    ///
    /// Each dependency is taken by value so the loops-band registry owns what it
    /// consumes at construction rather than reaching into the server-band
    /// composition root; the root destructures them out of `Ports` at the call.
    pub async fn from_config(
        config: &moatless_core::config::Config,
        pool: DbPool,
        provider_tokens: Arc<dyn ProviderTokenDao>,
        provider_token_auth: Arc<dyn ProviderTokenAuthDao>,
        github_apps: Arc<GitHubAppRegistry>,
        github_app_service: Option<Arc<GitHubAppService>>,
        // Needed so the GitHub adapter can resolve `secret_ref` provider tokens
        // (validated PATs live in `secrets`, not `provider_tokens`).
        secrets: Arc<SecretService>,
    ) -> (Self, ProviderHandles) {
        let mut providers = Self::new();

        let db = pool;

        let runtime_config = Arc::new(config.clone());
        let slack = Arc::new(crate::adapters::slack::adapter::SlackAdapter::from_db(
            runtime_config.clone(),
            db.clone(),
        ));
        providers.register_slack(slack.clone());
        tracing::info!("Slack adapter registered");

        // Webhook adapter is always available (config is per subscription).
        providers.register_webhook(Arc::new(
            crate::adapters::webhook::adapter::WebhookAdapter::new(),
        ));
        tracing::info!("Webhook adapter configured");

        let telegram = Arc::new(
            crate::adapters::telegram::adapter::TelegramAdapter::from_db(
                runtime_config,
                db.clone(),
            ),
        );
        providers.register_telegram(telegram.clone());
        tracing::info!("Telegram adapter registered");

        // GitHub PR adapter is always available; config is per subscription.
        let github_pr = Arc::new(
            crate::adapters::github_pr::adapter::GithubPrAdapter::new(
                config.github_pr.webhook_secret.clone(),
                config.github_pr.bot_token.clone(),
                config.auth.frontend_url.clone(),
                provider_tokens.clone(),
                Some(provider_token_auth.clone()),
                github_app_service.clone(),
                github_apps.clone(),
            )
            .with_secrets(secrets.clone()),
        );
        providers.register_github_pr(github_pr.clone());
        tracing::info!("GitHub PR adapter configured");

        // Linear adapter — workspace tokens come from the OAuth install
        // flow (provider_tokens, team_id=<linear org uuid>). OAuth client
        // credentials are needed to refresh expired access tokens.
        let linear = Arc::new(crate::adapters::linear::adapter::LinearAdapter::from_db(
            Arc::new(config.clone()),
            db,
        ));
        providers.register_linear(linear.clone());
        tracing::info!("Linear adapter configured");

        let handles = ProviderHandles {
            github_pr: Some(github_pr),
            linear: Some(linear),
            slack: Some(slack),
            telegram: Some(telegram),
        };
        (providers, handles)
    }
}
