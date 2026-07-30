//! Adapter registry construction from application configuration.

use std::sync::Arc;

use crate::auth::github_app_registry::GitHubAppRegistry;
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
        use crate::auth::provider_token_dao::{PgProviderTokenRepository, ProviderTokenRepository};
        use crate::auth::provider_tokens::{
            PgProviderTokenAuthRepository, ProviderTokenAuthRepository,
        };
        use crate::secret::audit::{PgSecretAuditRepository, SecretAuditRepository};
        use crate::secret::dao::{PgSecretRepository, SecretRepository};
        use crate::secret::service::{LocalEnvelopeSecretCipher, SecretService};

        let mut providers = Self::new();

        let provider_tokens: Arc<dyn ProviderTokenRepository> =
            Arc::new(PgProviderTokenRepository(db.clone()));
        let provider_token_auth: Arc<dyn ProviderTokenAuthRepository> =
            Arc::new(PgProviderTokenAuthRepository(db.clone()));
        let github_app_service = github_apps.default_service();

        // Needed so the GitHub adapter can resolve `secret_ref` provider tokens
        // (validated PATs live in `secrets`, not `provider_tokens`).
        let secret_records: Arc<dyn SecretRepository> = Arc::new(PgSecretRepository(db.clone()));
        let secret_audits: Arc<dyn SecretAuditRepository> =
            Arc::new(PgSecretAuditRepository(db.clone()));
        let secrets = Arc::new(SecretService::new(
            secret_records,
            secret_audits,
            LocalEnvelopeSecretCipher::from_config(&config.secrets),
        ));

        if let (Some(ref token), Some(ref secret)) =
            (&config.slack.bot_token, &config.slack.signing_secret)
        {
            let bot_user_id =
                resolve_slack_bot_user_id(token, config.slack.bot_user_id.as_deref()).await;
            let slack =
                crate::adapters::slack::adapter::SlackAdapter::new(token, secret, bot_user_id);
            providers.register_slack(Arc::new(slack));
            tracing::info!("Slack adapter configured");
        }

        // Webhook adapter is always available (config is per trigger source).
        providers.register_webhook(Arc::new(
            crate::adapters::webhook::adapter::WebhookAdapter::new(),
        ));
        tracing::info!("Webhook adapter configured");

        if let Some(ref token) = config.telegram.bot_token {
            let telegram = crate::adapters::telegram::adapter::TelegramAdapter::new(token);
            providers.register_telegram(Arc::new(telegram));
            tracing::info!("Telegram adapter configured");
        }

        // GitHub PR adapter is always available; config is per trigger source.
        {
            let github_pr = crate::adapters::github_pr::adapter::GithubPrAdapter::new(
                config.github_pr.webhook_secret.clone(),
                config.github_pr.bot_token.clone(),
                config.auth.frontend_url.clone(),
                db.clone(),
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
            let linear = crate::adapters::linear::adapter::LinearAdapter::new(
                config.linear.webhook_secret.clone(),
                config.linear.oauth_client_id.clone(),
                config.linear.oauth_client_secret.clone(),
                db,
            );
            providers.register_linear(Arc::new(linear));
            tracing::info!("Linear adapter configured");
        }

        Some(providers)
    }
}

/// Resolve the bot's Slack user ID.
///
/// Prefers the explicit env-var value; when absent, calls `auth.test` so the
/// agent's system prompt always has the bot's identity available.
async fn resolve_slack_bot_user_id(bot_token: &str, configured: Option<&str>) -> Option<String> {
    if let Some(id) = configured {
        let id = id.trim();
        if !id.is_empty() {
            tracing::info!(bot_user_id = %id, "Slack bot identity from env var");
            return Some(id.to_string());
        }
    }

    let api = crate::adapters::slack::api::SlackApi::new(bot_token);
    match api.auth_test().await {
        Ok(info) => {
            tracing::info!(
                bot_user_id = %info.user_id,
                handle = %info.user.as_deref().unwrap_or(""),
                team = %info.team.as_deref().unwrap_or(""),
                "Slack bot identity resolved via auth.test"
            );
            Some(info.user_id)
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                "Could not resolve Slack bot identity — set SLACK_BOT_USER_ID explicitly; \
                 agent system prompt will fall back to an unknown-identity message"
            );
            None
        }
    }
}
