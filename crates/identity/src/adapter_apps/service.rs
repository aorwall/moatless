//! Adapter app validation and encryption operations.

use super::models::{
    AdapterAppInput, AdapterAppRow, LoadedAdapterApp, SealedAdapterApp, StoredAdapterApp,
};
use crate::{
    auth::error::AuthError,
    secret::{
        models::EncryptedSecretValue,
        service::{LocalEnvelopeSecretCipher, SecretCipher},
    },
};
use moatless_adapter_model::apps::AdapterAppSecrets;
use moatless_core::{config::Config, db::DbPool};
use moatless_types::secret::SecretsConfig;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use tracing::warn;

pub(super) fn secret_fingerprint(plaintext: &str) -> String {
    let digest = Sha256::digest(plaintext.trim().as_bytes());
    hex::encode(&digest[..8])
}

fn secret_aad(app_id: &str) -> &str {
    app_id
}

pub fn app_id(adapter_kind: &str, app_key: &str) -> String {
    format!("aa_{adapter_kind}_{app_key}")
}

pub fn require_cipher(
    secrets: &SecretsConfig,
) -> Result<std::sync::Arc<dyn SecretCipher>, AuthError> {
    LocalEnvelopeSecretCipher::from_config(secrets).ok_or_else(|| {
        AuthError::Config(
            "DB-backed Adapter app config requires SECRETS_ENCRYPTION_KEY to store secrets"
                .to_string(),
        )
    })
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
    let input = seal(input, cipher)?;
    super::app_dao::upsert(db, input, actor_id).await
}

fn seal(input: AdapterAppInput, cipher: &dyn SecretCipher) -> Result<SealedAdapterApp, AuthError> {
    validate_adapter_app(&input)?;
    let id = app_id(input.adapter_kind, &input.app_key);
    let fingerprints = input
        .secrets
        .iter()
        .map(|secret| {
            (
                secret.name.to_string(),
                Value::String(secret_fingerprint(&secret.plaintext)),
            )
        })
        .collect();
    let secrets = input
        .secrets
        .into_iter()
        .map(|secret| {
            let value = cipher
                .encrypt(secret_aad(&id), &secret.plaintext)
                .map_err(|e| {
                    AuthError::Config(format!("Failed to encrypt Adapter app secret: {e}"))
                })?;
            Ok((secret.name, value))
        })
        .collect::<Result<_, AuthError>>()?;
    Ok(SealedAdapterApp {
        adapter_kind: input.adapter_kind,
        app_key: input.app_key,
        display_name: input.display_name,
        config_json: input.config_json,
        fingerprints,
        secrets,
    })
}

pub async fn carry_over_one(
    db: &DbPool,
    cipher: &dyn SecretCipher,
    marker: &'static str,
    input: AdapterAppInput,
) {
    if matches!(super::app_dao::has_marker(db, marker).await, Ok(true)) {
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

    let input = match seal(input, cipher) {
        Ok(input) => input,
        Err(error) => {
            warn!(marker, %error, "Could not seal Adapter app carry-over");
            return;
        }
    };
    super::app_dao::carry_over(db, marker, input).await;
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
    let Some((row, secret_rows)) = super::app_dao::read_secrets(db, adapter_kind, app_key).await?
    else {
        return Ok(None);
    };
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

pub async fn load_adapter_app(
    db: &DbPool,
    config: &Config,
    adapter_kind: &str,
    app_key: &str,
) -> Result<Option<LoadedAdapterApp>, AuthError> {
    Ok(app_secrets(db, config, adapter_kind, app_key)
        .await?
        .map(|(row, secrets)| LoadedAdapterApp {
            config_json: row.config_json,
            revision: row.revision,
            secrets,
        }))
}

/// Validate and seal an app before handing it to persistence.
pub(crate) async fn save_adapter_app(
    dao: &dyn super::dao::AdapterAppDao,
    cipher: &dyn SecretCipher,
    input: AdapterAppInput,
    actor_id: &str,
) -> Result<StoredAdapterApp, AuthError> {
    dao.upsert_adapter_app(seal(input, cipher)?, actor_id).await
}
