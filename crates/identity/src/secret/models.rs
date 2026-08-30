//! Secret domain types and validation helpers.

use chrono::{DateTime, Utc};
use moatless_types::secret::SecretSource;

/// Persisted secret row. The plaintext value is intentionally absent.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct SecretRecord {
    pub id: String,
    pub scope: String,
    pub owner_user_id: Option<String>,
    pub key: String,
    pub kind: String,
    pub provider: Option<String>,
    pub value_ciphertext: Option<Vec<u8>>,
    pub encryption_nonce: Option<Vec<u8>>,
    pub encrypted_data_key: Option<Vec<u8>>,
    pub external_ref: Option<String>,
    pub kms_key_id: Option<String>,
    pub encryption_version: i32,
    pub created_by: String,
    pub updated_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub version: i32,
    pub enabled: bool,
}

impl SecretRecord {
    pub fn source(&self) -> SecretSource {
        if self.external_ref.is_some() {
            SecretSource::ExternalRef
        } else {
            SecretSource::InlineEncrypted
        }
    }
}

/// Resolved plaintext secret for internal runtime consumers only.
#[derive(Debug, Clone)]
pub struct ResolvedSecret {
    pub key: String,
    pub value: String,
    pub provider: Option<String>,
    /// Raw `kind` string from the record (e.g. `"env"`, `"ssh_key"`). Lets
    /// callers route `ssh_key` secrets to `~/.ssh` instead of the env-var map.
    pub kind: String,
}

/// Storage payload for an encrypted inline secret.
#[derive(Debug, Clone)]
pub struct EncryptedSecretValue {
    pub value_ciphertext: Vec<u8>,
    pub encryption_nonce: Vec<u8>,
    pub encrypted_data_key: Vec<u8>,
    pub kms_key_id: String,
    pub encryption_version: i32,
}
