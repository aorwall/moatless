//! Adapter app inputs, persistence rows and loaded values.

use chrono::{DateTime, Utc};
use serde_json::Value;
use std::collections::HashMap;

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
#[derive(sqlx::FromRow)]
pub(super) struct AdapterAppRow {
    pub id: String,
    pub adapter_kind: String,
    pub app_key: String,
    pub display_name: String,
    pub config_json: Value,
    pub revision: i32,
    pub updated_at: DateTime<Utc>,
}
#[derive(sqlx::FromRow)]
pub(super) struct AdapterAppSecretRow {
    pub name: String,
    pub value_ciphertext: Vec<u8>,
    pub value_nonce: Vec<u8>,
    pub encrypted_data_key: Vec<u8>,
    pub kms_key_id: String,
    pub encryption_version: i32,
}

/// Validated app configuration with secrets sealed for persistence.
pub struct SealedAdapterApp {
    pub(crate) adapter_kind: &'static str,

    pub(crate) app_key: String,

    pub(crate) display_name: String,

    pub(crate) config_json: Value,

    pub(crate) fingerprints: Value,

    pub(crate) secrets: Vec<(&'static str, crate::secret::models::EncryptedSecretValue)>,
}

/// Decrypted runtime configuration for one registered app.
pub struct LoadedAdapterApp {
    pub config_json: Value,

    pub secrets: HashMap<String, String>,

    pub revision: i32,
}
