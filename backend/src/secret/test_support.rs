//! In-memory secret store for unit tests.
//!
//! `SecretRepository` has no `automock` (mockall can't synthesise the borrowed
//! `SecretInsert<'_>` / `SecretReplace<'_>` lifetimes), so tests that need a
//! `SecretService` build one over this hand-written stub.

use std::sync::Arc;

use super::audit::{SecretAuditInput, SecretAuditRepository};
use super::dao::{SecretInsert, SecretReplace, SecretRepository};
use super::error::SecretError;
use super::models::{EncryptedSecretValue, SecretRecord};
use super::service::{SecretCipher, SecretService};
use moatless_backend_core::scope::ScopeContext;

/// Read-only in-memory store. Write methods are unimplemented — tests that
/// exercise writes should use the real DAO via `#[sqlx::test]`.
pub struct InMemorySecretStore {
    pub records: Vec<SecretRecord>,
}

#[async_trait::async_trait]
impl SecretRepository for InMemorySecretStore {
    async fn list(
        &self,
        _ctx: &ScopeContext,
        scope_filter: Option<&str>,
    ) -> Result<Vec<SecretRecord>, sqlx::Error> {
        Ok(self
            .records
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
        Ok(self.records.iter().find(|r| r.id == id).cloned())
    }

    async fn find_by_key(
        &self,
        scope: &str,
        owner_user_id: Option<&str>,
        key: &str,
    ) -> Result<Option<SecretRecord>, sqlx::Error> {
        Ok(self
            .records
            .iter()
            .find(|r| {
                r.scope == scope && r.owner_user_id.as_deref() == owner_user_id && r.key == key
            })
            .cloned())
    }

    async fn insert(&self, _input: SecretInsert<'_>) -> Result<SecretRecord, sqlx::Error> {
        unimplemented!("InMemorySecretStore is read-only")
    }

    async fn replace(
        &self,
        _input: SecretReplace<'_>,
        _ctx: &ScopeContext,
    ) -> Result<Option<SecretRecord>, sqlx::Error> {
        unimplemented!("InMemorySecretStore is read-only")
    }

    async fn delete(&self, _id: &str, _ctx: &ScopeContext) -> Result<bool, sqlx::Error> {
        unimplemented!("InMemorySecretStore is read-only")
    }

    async fn mark_used(&self, _id: &str) -> Result<(), sqlx::Error> {
        Ok(())
    }
}

/// Audit sink that drops every event.
pub struct NoopAudits;

#[async_trait::async_trait]
impl SecretAuditRepository for NoopAudits {
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
        _plaintext: &str,
    ) -> Result<EncryptedSecretValue, SecretError> {
        unimplemented!("PlaintextCipher does not encrypt")
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
        Arc::new(InMemorySecretStore { records }),
        Arc::new(NoopAudits),
        Some(Arc::new(PlaintextCipher)),
    )
}
