//! Deployment-wide Adapter app/API configuration stored in the database.
//!
//! These rows replace Helm/env values for Adapter integrations while keeping
//! auth-provider configuration out of the admin-editable database path.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::Row;
use tracing::{info, warn};

use crate::auth::error::AuthError;
use crate::secret::models::EncryptedSecretValue;
use crate::secret::service::{LocalEnvelopeSecretCipher, SecretCipher};
use moatless_adapter_model::apps::AdapterAppSecrets;
use moatless_core::config::{Config, SlackConfig, TelegramConfig};
use moatless_types::adapter::LinearConfig;
use moatless_types::secret::SecretsConfig;
use moatless_core::db::DbPool;

pub mod dao;

pub use moatless_adapter_model::apps::{DEFAULT_APP_KEY, app_key_from_transport_config};
const SYSTEM_ACTOR: &str = "adapter_apps_env_carry_over";

#[derive(Clone)]
pub struct AdapterAppSecretInput {
    pub name: &'static str,
    pub plaintext: String,
}

#[derive(Clone)]
pub struct AdapterAppInput {
    pub adapter_kind: &'static str,
    pub app_key: String,
    pub display_name: String,
    pub config_json: Value,
    pub secrets: Vec<AdapterAppSecretInput>,
}

#[derive(sqlx::FromRow)]
struct AdapterAppRow {
    id: String,
    adapter_kind: String,
    app_key: String,
    display_name: String,
    config_json: Value,
    revision: i32,
    updated_at: DateTime<Utc>,
}

#[derive(sqlx::FromRow)]
struct AdapterAppSecretRow {
    name: String,
    value_ciphertext: Vec<u8>,
    value_nonce: Vec<u8>,
    encrypted_data_key: Vec<u8>,
    kms_key_id: String,
    encryption_version: i32,
}

#[derive(Clone, Debug)]
pub struct StoredAdapterApp {
    pub id: String,
    pub adapter_kind: String,
    pub app_key: String,
    pub display_name: String,
    pub config_json: Value,
    pub secret_fingerprints: HashMap<String, String>,
    pub revision: i32,
    pub updated_at: DateTime<Utc>,
}

fn secret_fingerprint(plaintext: &str) -> String {
    let digest = Sha256::digest(plaintext.trim().as_bytes());
    hex::encode(&digest[..8])
}

fn secret_aad(app_id: &str) -> &str {
    app_id
}

pub fn app_id(adapter_kind: &str, app_key: &str) -> String {
    format!("aa_{adapter_kind}_{app_key}")
}

fn require_cipher(secrets: &SecretsConfig) -> Result<std::sync::Arc<dyn SecretCipher>, AuthError> {
    LocalEnvelopeSecretCipher::from_config(secrets).ok_or_else(|| {
        AuthError::Config(
            "DB-backed Adapter app config requires SECRETS_ENCRYPTION_KEY to store secrets"
                .to_string(),
        )
    })
}

async fn has_marker(db: &DbPool, marker: &str) -> Result<bool, AuthError> {
    let row: (bool,) =
        sqlx::query_as("SELECT EXISTS (SELECT 1 FROM adapter_app_migrations WHERE name = $1)")
            .bind(marker)
            .fetch_one(db)
            .await?;
    Ok(row.0)
}

async fn insert_app(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    input: &AdapterAppInput,
    cipher: &dyn SecretCipher,
    actor_id: &str,
) -> Result<bool, AuthError> {
    let id = app_id(input.adapter_kind, &input.app_key);
    let fingerprints: Value = input
        .secrets
        .iter()
        .map(|secret| {
            (
                secret.name.to_string(),
                Value::String(secret_fingerprint(&secret.plaintext)),
            )
        })
        .collect();

    let inserted = sqlx::query(
        r#"INSERT INTO adapter_apps (
               id, adapter_kind, app_key, display_name, enabled, config_json,
               secret_fingerprints, created_by, updated_by, revision
           )
           VALUES ($1, $2, $3, $4, true, $5, $6, $7, $7, 1)
           ON CONFLICT (adapter_kind, app_key) DO NOTHING"#,
    )
    .bind(&id)
    .bind(input.adapter_kind)
    .bind(&input.app_key)
    .bind(&input.display_name)
    .bind(&input.config_json)
    .bind(&fingerprints)
    .bind(actor_id)
    .execute(&mut **tx)
    .await?;

    if inserted.rows_affected() == 0 {
        return Ok(false);
    }

    for secret in &input.secrets {
        let encrypted = cipher
            .encrypt(secret_aad(&id), &secret.plaintext)
            .map_err(|e| AuthError::Config(format!("Failed to encrypt Adapter app secret: {e}")))?;
        sqlx::query(
            r#"INSERT INTO adapter_app_secrets (
                   adapter_app_id, name, value_ciphertext, value_nonce,
                   encrypted_data_key, kms_key_id, encryption_version
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        )
        .bind(&id)
        .bind(secret.name)
        .bind(&encrypted.value_ciphertext)
        .bind(&encrypted.encryption_nonce)
        .bind(&encrypted.encrypted_data_key)
        .bind(&encrypted.kms_key_id)
        .bind(encrypted.encryption_version)
        .execute(&mut **tx)
        .await?;
    }
    Ok(true)
}

pub async fn upsert_adapter_app(
    db: &DbPool,
    secrets: &SecretsConfig,
    input: AdapterAppInput,
    actor_id: &str,
) -> Result<StoredAdapterApp, AuthError> {
    let cipher = require_cipher(secrets)?;
    upsert_adapter_app_with_cipher(db, cipher.as_ref(), input, actor_id).await
}

pub(crate) async fn upsert_adapter_app_with_cipher(
    db: &DbPool,
    cipher: &dyn SecretCipher,
    input: AdapterAppInput,
    actor_id: &str,
) -> Result<StoredAdapterApp, AuthError> {
    validate_adapter_app(&input)?;
    let id = app_id(input.adapter_kind, &input.app_key);
    let fingerprints: Value = input
        .secrets
        .iter()
        .map(|secret| {
            (
                secret.name.to_string(),
                Value::String(secret_fingerprint(&secret.plaintext)),
            )
        })
        .collect();

    let mut tx = db.begin().await?;
    sqlx::query(
        r#"INSERT INTO adapter_apps (
               id, adapter_kind, app_key, display_name, enabled, config_json,
               secret_fingerprints, created_by, updated_by, revision
           )
           VALUES ($1, $2, $3, $4, true, $5, $6, $7, $7, 1)
           ON CONFLICT (adapter_kind, app_key) DO UPDATE SET
               display_name = EXCLUDED.display_name,
               enabled = true,
               config_json = EXCLUDED.config_json,
               secret_fingerprints = EXCLUDED.secret_fingerprints,
               updated_by = EXCLUDED.updated_by,
               updated_at = now(),
               revision = adapter_apps.revision + 1"#,
    )
    .bind(&id)
    .bind(input.adapter_kind)
    .bind(&input.app_key)
    .bind(&input.display_name)
    .bind(&input.config_json)
    .bind(&fingerprints)
    .bind(actor_id)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM adapter_app_secrets WHERE adapter_app_id = $1")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    for secret in &input.secrets {
        let encrypted = cipher
            .encrypt(secret_aad(&id), &secret.plaintext)
            .map_err(|e| AuthError::Config(format!("Failed to encrypt Adapter app secret: {e}")))?;
        sqlx::query(
            r#"INSERT INTO adapter_app_secrets (
                   adapter_app_id, name, value_ciphertext, value_nonce,
                   encrypted_data_key, kms_key_id, encryption_version
               )
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
        )
        .bind(&id)
        .bind(secret.name)
        .bind(&encrypted.value_ciphertext)
        .bind(&encrypted.encryption_nonce)
        .bind(&encrypted.encrypted_data_key)
        .bind(&encrypted.kms_key_id)
        .bind(encrypted.encryption_version)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    find_adapter_app(db, input.adapter_kind, &input.app_key)
        .await?
        .ok_or_else(|| AuthError::Internal("saved Adapter app was not readable".to_string()))
}

pub async fn list_adapter_apps(db: &DbPool) -> Result<Vec<StoredAdapterApp>, AuthError> {
    let rows: Vec<AdapterAppRow> = sqlx::query_as(
        r#"SELECT id, adapter_kind, app_key, display_name, config_json, revision, updated_at
           FROM adapter_apps
           ORDER BY adapter_kind, app_key"#,
    )
    .fetch_all(db)
    .await?;

    let mut apps = Vec::with_capacity(rows.len());
    for row in rows {
        apps.push(stored_app_from_row(db, row).await?);
    }
    Ok(apps)
}

pub async fn find_adapter_app(
    db: &DbPool,
    adapter_kind: &str,
    app_key: &str,
) -> Result<Option<StoredAdapterApp>, AuthError> {
    let row: Option<AdapterAppRow> = sqlx::query_as(
        r#"SELECT id, adapter_kind, app_key, display_name, config_json, revision, updated_at
           FROM adapter_apps
           WHERE adapter_kind = $1 AND app_key = $2"#,
    )
    .bind(adapter_kind)
    .bind(app_key)
    .fetch_optional(db)
    .await?;
    match row {
        Some(row) => Ok(Some(stored_app_from_row(db, row).await?)),
        None => Ok(None),
    }
}

async fn stored_app_from_row(
    db: &DbPool,
    row: AdapterAppRow,
) -> Result<StoredAdapterApp, AuthError> {
    let secret_fingerprints: HashMap<String, String> =
        sqlx::query("SELECT secret_fingerprints FROM adapter_apps WHERE id = $1")
            .bind(&row.id)
            .fetch_one(db)
            .await?
            .try_get::<Value, _>("secret_fingerprints")
            .unwrap_or_else(|_| json!({}))
            .as_object()
            .map(|object| {
                object
                    .iter()
                    .filter_map(|(key, value)| value.as_str().map(|v| (key.clone(), v.to_string())))
                    .collect()
            })
            .unwrap_or_default();
    Ok(StoredAdapterApp {
        id: row.id,
        adapter_kind: row.adapter_kind,
        app_key: row.app_key,
        display_name: row.display_name,
        config_json: row.config_json,
        secret_fingerprints,
        revision: row.revision,
        updated_at: row.updated_at,
    })
}

pub async fn delete_adapter_app(
    db: &DbPool,
    adapter_kind: &str,
    app_key: &str,
) -> Result<bool, AuthError> {
    let deleted = sqlx::query("DELETE FROM adapter_apps WHERE adapter_kind = $1 AND app_key = $2")
        .bind(adapter_kind)
        .bind(app_key)
        .execute(db)
        .await?;
    Ok(deleted.rows_affected() > 0)
}

fn validate_adapter_app(input: &AdapterAppInput) -> Result<(), AuthError> {
    let valid_kind = !input.adapter_kind.is_empty()
        && input.adapter_kind.len() <= 32
        && input
            .adapter_kind
            .starts_with(|c: char| c.is_ascii_lowercase())
        && input
            .adapter_kind
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !valid_kind {
        return Err(AuthError::BadRequest(format!(
            "'{}' is not a usable Adapter kind",
            input.adapter_kind
        )));
    }
    let valid_key = !input.app_key.is_empty()
        && input.app_key.len() <= 64
        && input
            .app_key
            .starts_with(|c: char| c.is_ascii_alphanumeric())
        && input
            .app_key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid_key {
        return Err(AuthError::BadRequest(format!(
            "'{}' is not a usable Adapter app key",
            input.app_key
        )));
    }
    Ok(())
}

async fn app_secrets(
    db: &DbPool,
    config: &Config,
    adapter_kind: &str,
    app_key: &str,
) -> Result<Option<(AdapterAppRow, HashMap<String, String>)>, AuthError> {
    let Some(cipher) = LocalEnvelopeSecretCipher::from_config(&config.secrets) else {
        warn!(
            adapter_kind,
            "Cannot read DB-backed Adapter app config without SECRETS_ENCRYPTION_KEY"
        );
        return Ok(None);
    };
    let row: Option<AdapterAppRow> = sqlx::query_as(
        r#"SELECT id, adapter_kind, app_key, display_name, config_json, revision, updated_at
           FROM adapter_apps
           WHERE adapter_kind = $1 AND app_key = $2 AND enabled = true"#,
    )
    .bind(adapter_kind)
    .bind(app_key)
    .fetch_optional(db)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };

    let secret_rows: Vec<AdapterAppSecretRow> = sqlx::query_as(
        r#"SELECT name, value_ciphertext, value_nonce, encrypted_data_key,
                  kms_key_id, encryption_version
           FROM adapter_app_secrets
           WHERE adapter_app_id = $1"#,
    )
    .bind(&row.id)
    .fetch_all(db)
    .await?;
    let mut secrets = HashMap::new();
    for secret in secret_rows {
        let value = cipher
            .decrypt_value(
                secret_aad(&row.id),
                &EncryptedSecretValue {
                    value_ciphertext: secret.value_ciphertext,
                    encryption_nonce: secret.value_nonce,
                    encrypted_data_key: secret.encrypted_data_key,
                    kms_key_id: secret.kms_key_id,
                    encryption_version: secret.encryption_version,
                },
            )
            .map_err(|e| {
                AuthError::Config(format!(
                    "Failed to decrypt {adapter_kind} Adapter app secret '{}': {e}",
                    secret.name
                ))
            })?;
        secrets.insert(secret.name, value);
    }
    Ok(Some((row, secrets)))
}

/// One stored app, narrowed to what an Adapter reads. The row type stays
/// private to this module; the port hands out this shape.
pub(crate) async fn stored_app(
    db: &DbPool,
    config: &Config,
    adapter_kind: &str,
    app_key: &str,
) -> Result<Option<AdapterAppSecrets>, AuthError> {
    Ok(app_secrets(db, config, adapter_kind, app_key)
        .await?
        .map(|(row, secrets)| AdapterAppSecrets {
            app_key: row.app_key,
            secrets,
            config: row.config_json,
        }))
}

async fn default_app_secrets(
    db: &DbPool,
    config: &Config,
    adapter_kind: &str,
) -> Result<Option<(AdapterAppRow, HashMap<String, String>)>, AuthError> {
    app_secrets(db, config, adapter_kind, DEFAULT_APP_KEY).await
}

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

async fn carry_over_one(
    db: &DbPool,
    cipher: &dyn SecretCipher,
    marker: &'static str,
    input: AdapterAppInput,
) {
    if matches!(has_marker(db, marker).await, Ok(true)) {
        return;
    }
    if let Err(e) = validate_adapter_app(&input) {
        warn!(
            adapter_kind = input.adapter_kind,
            app_key = %input.app_key,
            error = %e,
            "Not carrying over Adapter app env config"
        );
        return;
    }

    let mut tx = match db.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            warn!(error = %e, "Could not start Adapter app carry-over transaction");
            return;
        }
    };
    let claimed = match sqlx::query(
        r#"INSERT INTO adapter_app_migrations (name, detail)
           VALUES ($1, $2)
           ON CONFLICT (name) DO NOTHING"#,
    )
    .bind(marker)
    .bind(format!(
        "{}:{} carried over from environment",
        input.adapter_kind, input.app_key
    ))
    .execute(&mut *tx)
    .await
    {
        Ok(done) => done.rows_affected() > 0,
        Err(e) => {
            warn!(marker, error = %e, "Could not claim Adapter app carry-over marker");
            let _ = tx.rollback().await;
            return;
        }
    };
    if !claimed {
        let _ = tx.rollback().await;
        return;
    }

    match insert_app(&mut tx, &input, cipher, SYSTEM_ACTOR).await {
        Ok(inserted) => {
            if let Err(e) = tx.commit().await {
                warn!(marker, error = %e, "Could not commit Adapter app carry-over");
                return;
            }
            if inserted {
                info!(
                    adapter_kind = input.adapter_kind,
                    app_key = %input.app_key,
                    "Carried Adapter app env config into DB"
                );
            } else {
                warn!(
                    adapter_kind = input.adapter_kind,
                    app_key = %input.app_key,
                    "Skipped Adapter app env carry-over because that app is already registered"
                );
            }
        }
        Err(e) => {
            warn!(
                marker,
                adapter_kind = input.adapter_kind,
                app_key = %input.app_key,
                error = %e,
                "Could not carry Adapter app env config into DB"
            );
            let _ = tx.rollback().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_APP_KEY, app_key_from_transport_config};

    #[test]
    fn transport_config_app_key_defaults_when_absent_or_blank() {
        assert_eq!(
            app_key_from_transport_config(&serde_json::json!({})),
            DEFAULT_APP_KEY
        );
        assert_eq!(
            app_key_from_transport_config(&serde_json::json!({ "app_key": "   " })),
            DEFAULT_APP_KEY
        );
        assert_eq!(
            app_key_from_transport_config(&serde_json::json!({ "app_key": "enterprise" })),
            "enterprise"
        );
    }
}
