//! In-memory secret store for unit tests.
//!
//! `SecretDao` has no `automock` (mockall can't synthesise the borrowed
//! `SecretInsert<'_>` / `SecretReplace<'_>` lifetimes), so tests that need a
//! `SecretService` build one over this hand-written stub.

// Under the `test-support` feature this module is part of the library rather than
// a `#[cfg(test)]` target, so the library's lints apply to it and are waived here.
#![allow(
    clippy::expect_used,
    reason = "a broken test fixture should fail the test loudly, not be handled"
)]

use std::sync::Arc;

use super::audit::{SecretAuditDao, SecretAuditInput};
use super::dao::{SecretDao, SecretInsert, SecretReplace};
use super::error::SecretError;
use super::models::{EncryptedSecretValue, SecretRecord};
use super::service::{SecretCipher, SecretService};
use moatless_types::access_control::ScopeContext;

/// In-memory store, readable and writable.
///
/// Writes are here so a test can exercise a flow that stores a secret and then
/// reads it back — stashing a displaced GitHub token behind a PAT is the case
/// that needs it. It enforces none of the real DAO's constraints, so a test
/// about SQL behaviour still belongs on `#[sqlx::test]`.
pub struct InMemorySecretDao {
    pub records: std::sync::Mutex<Vec<SecretRecord>>,
}

#[async_trait::async_trait]
impl SecretDao for InMemorySecretDao {
    async fn list(
        &self,
        _ctx: &ScopeContext,
        scope_filter: Option<&str>,
    ) -> Result<Vec<SecretRecord>, sqlx::Error> {
        Ok(self
            .rows()
            .iter()
            .filter(|r| scope_filter.is_none_or(|s| r.scope == s))
            .cloned()
            .collect())
    }

    async fn find_by_id(
        &self,
        id: &str,
        _ctx: &ScopeContext,
    ) -> Result<Option<SecretRecord>, sqlx::Error> {
        Ok(self.rows().iter().find(|r| r.id == id).cloned())
    }

    async fn find_by_key(
        &self,
        scope: &str,
        owner_user_id: Option<&str>,
        key: &str,
    ) -> Result<Option<SecretRecord>, sqlx::Error> {
        Ok(self
            .rows()
            .iter()
            .find(|r| {
                r.scope == scope && r.owner_user_id.as_deref() == owner_user_id && r.key == key
            })
            .cloned())
    }

    async fn insert(&self, input: SecretInsert<'_>) -> Result<SecretRecord, sqlx::Error> {
        let now = chrono::Utc::now();
        let record = SecretRecord {
            id: input.id.to_string(),
            scope: input.scope.to_string(),
            owner_user_id: input.owner_user_id.map(str::to_string),
            key: input.key.to_string(),
            kind: input.kind.to_string(),
            provider: input.provider.map(str::to_string),
            value_ciphertext: input.value_ciphertext.map(<[u8]>::to_vec),
            encryption_nonce: input.encryption_nonce.map(<[u8]>::to_vec),
            encrypted_data_key: input.encrypted_data_key.map(<[u8]>::to_vec),
            external_ref: input.external_ref.map(str::to_string),
            kms_key_id: input.kms_key_id.map(str::to_string),
            encryption_version: input.encryption_version,
            created_by: input.actor_id.to_string(),
            updated_by: input.actor_id.to_string(),
            created_at: now,
            updated_at: now,
            last_used_at: None,
            version: 1,
            enabled: true,
        };
        self.records.lock().expect("lock").push(record.clone());
        Ok(record)
    }

    async fn replace(
        &self,
        input: SecretReplace<'_>,
        _ctx: &ScopeContext,
    ) -> Result<Option<SecretRecord>, sqlx::Error> {
        let mut records = self.records.lock().expect("lock");
        let Some(record) = records.iter_mut().find(|r| r.id == input.id) else {
            return Ok(None);
        };
        record.kind = input.kind.to_string();
        record.provider = input.provider.map(str::to_string);
        record.value_ciphertext = input.value_ciphertext.map(<[u8]>::to_vec);
        record.encryption_nonce = input.encryption_nonce.map(<[u8]>::to_vec);
        record.encrypted_data_key = input.encrypted_data_key.map(<[u8]>::to_vec);
        record.external_ref = input.external_ref.map(str::to_string);
        record.kms_key_id = input.kms_key_id.map(str::to_string);
        record.encryption_version = input.encryption_version;
        record.updated_by = input.actor_id.to_string();
        record.updated_at = chrono::Utc::now();
        record.version += 1;
        if let Some(enabled) = input.enabled {
            record.enabled = enabled;
        }
        Ok(Some(record.clone()))
    }

    async fn delete(&self, id: &str, _ctx: &ScopeContext) -> Result<bool, sqlx::Error> {
        let mut records = self.records.lock().expect("lock");
        let before = records.len();
        records.retain(|r| r.id != id);
        Ok(records.len() != before)
    }

    async fn mark_used(&self, _id: &str) -> Result<(), sqlx::Error> {
        Ok(())
    }
}

impl InMemorySecretDao {
    fn rows(&self) -> std::sync::MutexGuard<'_, Vec<SecretRecord>> {
        self.records.lock().expect("lock")
    }
}

/// Audit sink that drops every event.
pub struct NoopAudits;

#[async_trait::async_trait]
impl SecretAuditDao for NoopAudits {
    async fn record(&self, _input: SecretAuditInput<'_>) -> Result<(), sqlx::Error> {
        Ok(())
    }
}

/// Cipher stub: the stored "ciphertext" bytes are the UTF-8 plaintext verbatim.
pub struct PlaintextCipher;

impl SecretCipher for PlaintextCipher {
    fn encrypt(
        &self,
        _secret_id: &str,
        plaintext: &str,
    ) -> Result<EncryptedSecretValue, SecretError> {
        Ok(EncryptedSecretValue {
            value_ciphertext: plaintext.as_bytes().to_vec(),
            encryption_nonce: Vec::new(),
            encrypted_data_key: Vec::new(),
            kms_key_id: String::new(),
            encryption_version: 1,
        })
    }

    fn decrypt_value(
        &self,
        _aad: &str,
        value: &EncryptedSecretValue,
    ) -> Result<String, SecretError> {
        Ok(String::from_utf8(value.value_ciphertext.clone())
            .expect("test ciphertext is valid utf8"))
    }

    fn decrypt(&self, record: &SecretRecord) -> Result<String, SecretError> {
        let bytes = record.value_ciphertext.clone().unwrap_or_default();
        Ok(String::from_utf8(bytes).expect("test ciphertext is valid utf8"))
    }
}

/// Build a `SecretRecord` whose ciphertext is the plaintext `value`.
pub fn secret(
    scope: &str,
    owner_user_id: Option<&str>,
    kind: &str,
    key: &str,
    value: &str,
) -> SecretRecord {
    SecretRecord {
        id: format!("{scope}:{key}"),
        scope: scope.to_string(),
        owner_user_id: owner_user_id.map(str::to_string),
        key: key.to_string(),
        kind: kind.to_string(),
        provider: None,
        value_ciphertext: Some(value.as_bytes().to_vec()),
        encryption_nonce: None,
        encrypted_data_key: None,
        external_ref: None,
        kms_key_id: None,
        encryption_version: 1,
        created_by: "test".to_string(),
        updated_by: "test".to_string(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        last_used_at: None,
        version: 1,
        enabled: true,
    }
}

/// Build a `SecretService` backed by `records` and the plaintext cipher.
pub fn service_with(records: Vec<SecretRecord>) -> SecretService {
    SecretService::new(
        Arc::new(InMemorySecretDao {
            records: std::sync::Mutex::new(records),
        }),
        Arc::new(NoopAudits),
        Some(Arc::new(PlaintextCipher)),
    )
}
