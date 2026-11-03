//! Provider configuration assembled from stored Adapter registrations.

use moatless_core::{
    config::{Config, SlackConfig},
    db::DbPool,
};
use moatless_identity::{
    adapter_apps::{DEFAULT_APP_KEY, LoadedAdapterApp, load_adapter_app},
    auth::error::AuthError,
};
use moatless_types::adapter::LinearConfig;
use serde_json::Value;
use tracing::info;

/// `config` with each Adapter's default app, as stored in `adapter_apps`.
pub async fn load_adapter_app_config(config: &Config, db: &DbPool) -> Result<Config, AuthError> {
    let mut next = config.clone();
    if let Some((row, secrets)) = default_app_secrets(db, config, "slack").await? {
        next.slack = SlackConfig {
            bot_token: secrets.get("bot_token").cloned(),
            signing_secret: secrets.get("signing_secret").cloned(),
            bot_user_id: row
                .config_json
                .get("bot_user_id")
                .and_then(Value::as_str)
                .map(str::to_string),
        };
        info!(
            revision = row.revision,
            "Loaded Slack Adapter app config from DB"
        );
    }
    if let Some((row, secrets)) = default_app_secrets(db, config, "linear").await? {
        next.linear = LinearConfig {
            webhook_secret: secrets.get("webhook_secret").cloned(),
            oauth_client_id: secrets.get("oauth_client_id").cloned(),
            oauth_client_secret: secrets.get("oauth_client_secret").cloned(),
            oauth_callback_url: row
                .config_json
                .get("oauth_callback_url")
                .and_then(Value::as_str)
                .map(str::to_string),
            api_base_url: row
                .config_json
                .get("api_base_url")
                .and_then(Value::as_str)
                .map(str::to_string),
        };
        info!(
            revision = row.revision,
            "Loaded Linear Adapter app config from DB"
        );
    }
    Ok(next)
}

async fn default_app_secrets(
    db: &DbPool,
    config: &Config,
    kind: &str,
) -> Result<Option<(LoadedAdapterApp, std::collections::HashMap<String, String>)>, AuthError> {
    Ok(load_adapter_app(db, config, kind, DEFAULT_APP_KEY)
        .await?
        .map(|mut app| {
            let secrets = std::mem::take(&mut app.secrets);
            (app, secrets)
        }))
}
