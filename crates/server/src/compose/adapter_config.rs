//! Provider configuration assembled from stored Adapter registrations.

use moatless_core::{
    config::{Config, SlackConfig, TelegramConfig},
    db::DbPool,
};
use moatless_identity::{
    adapter_apps::{
        AdapterAppInput, AdapterAppSecretInput, DEFAULT_APP_KEY, LoadedAdapterApp, carry_over_one,
        load_adapter_app, require_cipher,
    },
    auth::error::AuthError,
};
use moatless_types::adapter::LinearConfig;
use serde_json::{Value, json};
use tracing::{info, warn};

pub async fn carry_over_env_and_load_config(
    config: &Config,
    db: &DbPool,
) -> Result<Config, AuthError> {
    carry_over_env_adapter_apps(config, db).await;

    let mut next = config.clone();
    // Adapter app/API credentials are DB-authoritative after carry-over. The
    // env-derived values above are migration inputs only; leaving them on
    // `Config` would keep Helm/env as a permanent fallback and make deleting a
    // DB app row fail to disable that Adapter on the next restart.
    next.slack = SlackConfig {
        bot_token: None,
        signing_secret: None,
        bot_user_id: None,
    };
    next.telegram = TelegramConfig {
        bot_token: None,
        mode: "webhook".to_string(),
    };
    next.linear = LinearConfig {
        webhook_secret: None,
        oauth_client_id: None,
        oauth_client_secret: None,
        oauth_callback_url: None,
        api_base_url: None,
    };

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
    if let Some((row, secrets)) = default_app_secrets(db, config, "telegram").await? {
        next.telegram = TelegramConfig {
            bot_token: secrets.get("bot_token").cloned(),
            mode: row
                .config_json
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("webhook")
                .to_string(),
        };
        info!(
            revision = row.revision,
            "Loaded Telegram Adapter app config from DB"
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

async fn carry_over_env_adapter_apps(config: &Config, db: &DbPool) {
    let Ok(cipher) = require_cipher(&config.secrets) else {
        if config.slack.bot_token.is_some()
            || config.slack.signing_secret.is_some()
            || config.telegram.bot_token.is_some()
            || config.linear.webhook_secret.is_some()
            || config.linear.oauth_client_id.is_some()
            || config.linear.oauth_client_secret.is_some()
        {
            warn!(
                "Adapter app env config is set but cannot be carried over without SECRETS_ENCRYPTION_KEY"
            );
        }
        return;
    };

    if let (Some(bot_token), Some(signing_secret)) = (
        config.slack.bot_token.clone(),
        config.slack.signing_secret.clone(),
    ) {
        carry_over_one(
            db,
            cipher.as_ref(),
            "slack_env_carry_over_v1",
            AdapterAppInput {
                adapter_kind: "slack",
                app_key: DEFAULT_APP_KEY.to_string(),
                display_name: "Slack".to_string(),
                config_json: json!({ "bot_user_id": config.slack.bot_user_id }),
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
            },
        )
        .await;
    }

    if let Some(bot_token) = config.telegram.bot_token.clone() {
        carry_over_one(
            db,
            cipher.as_ref(),
            "telegram_env_carry_over_v1",
            AdapterAppInput {
                adapter_kind: "telegram",
                app_key: DEFAULT_APP_KEY.to_string(),
                display_name: "Telegram".to_string(),
                config_json: json!({ "mode": config.telegram.mode }),
                secrets: vec![AdapterAppSecretInput {
                    name: "bot_token",
                    plaintext: bot_token,
                }],
            },
        )
        .await;
    }

    if config.linear.webhook_secret.is_some()
        || config.linear.oauth_client_id.is_some()
        || config.linear.oauth_client_secret.is_some()
    {
        let mut secrets = Vec::new();
        if let Some(value) = config.linear.webhook_secret.clone() {
            secrets.push(AdapterAppSecretInput {
                name: "webhook_secret",
                plaintext: value,
            });
        }
        if let Some(value) = config.linear.oauth_client_id.clone() {
            secrets.push(AdapterAppSecretInput {
                name: "oauth_client_id",
                plaintext: value,
            });
        }
        if let Some(value) = config.linear.oauth_client_secret.clone() {
            secrets.push(AdapterAppSecretInput {
                name: "oauth_client_secret",
                plaintext: value,
            });
        }
        carry_over_one(
            db,
            cipher.as_ref(),
            "linear_env_carry_over_v1",
            AdapterAppInput {
                adapter_kind: "linear",
                app_key: DEFAULT_APP_KEY.to_string(),
                display_name: "Linear".to_string(),
                config_json: json!({
                    "oauth_callback_url": config.linear.oauth_callback_url,
                    "api_base_url": config.linear.api_base_url,
                }),
                secrets,
            },
        )
        .await;
    }
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
