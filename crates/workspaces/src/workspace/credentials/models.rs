//! Workspace credential domain types — DB row structs.

use chrono::{DateTime, Utc};

use moatless_identity::secret::models::EncryptedSecretValue;

use moatless_types::workspace::WorkspaceCredentialProvider;

/// Credential metadata safe to expose through the Workspace Adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceCredentialMetadata {
    pub workspace_id: String,
    pub provider: WorkspaceCredentialProvider,
    pub env_key: String,
    pub created_by: String,
    pub updated_by: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Row from the `workspace_credentials` table.
///
/// Carries ciphertext only; the plaintext exists solely inside
/// [`super::service::WorkspaceCredentialService`] and in the credential map
/// handed to a sandbox.
#[derive(sqlx::FromRow, Debug, Clone)]
pub struct WorkspaceCredential {
    pub id: String,
    pub workspace_id: String,
    /// Provider name as persisted. Kept as a string rather than the enum so a
    /// row written by a newer backend does not fail to deserialize on an older
    /// one; the service parses it and skips what it cannot map.
    pub provider: String,
    pub value_ciphertext: Vec<u8>,
    pub encryption_nonce: Vec<u8>,
    pub encrypted_data_key: Vec<u8>,
    pub kms_key_id: String,
    pub encryption_version: i32,
    pub created_by: String,
    pub updated_by: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl WorkspaceCredential {
    /// The stored value in the shape the cipher decrypts.
    pub fn encrypted_value(&self) -> EncryptedSecretValue {
        EncryptedSecretValue {
            value_ciphertext: self.value_ciphertext.clone(),
            encryption_nonce: self.encryption_nonce.clone(),
            encrypted_data_key: self.encrypted_data_key.clone(),
            kms_key_id: self.kms_key_id.clone(),
            encryption_version: self.encryption_version,
        }
    }
}

/// An upsert payload: the ciphertext plus who wrote it.
///
/// `id` is the additional authenticated data the value was encrypted under, so
/// it travels with the ciphertext; only a genuinely new row allocates one — an
/// update keeps the existing id and re-encrypts under it.
#[derive(Debug, Clone)]
pub struct WorkspaceCredentialWrite {
    pub id: String,
    pub workspace_id: String,
    pub provider: String,
    pub value: EncryptedSecretValue,
    pub actor_id: String,
}
