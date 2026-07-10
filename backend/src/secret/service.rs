//! Secret validation, access checks, encryption, and resolution.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::Engine;

use super::audit::{SecretAuditInput, SecretAuditRepository};
use super::dao::{SecretInsert, SecretReplace, SecretRepository};
use super::error::SecretError;
use super::models::{
    is_reserved_key, is_valid_secret_key, EncryptedSecretValue, ResolvedSecret, SecretKind,
    SecretRecord, SecretScopeType,
};
use super::schemas::{
    PatchSecretRequest, PutSecretRequest, SecretMetadataResponse, ValidateSecretsRequest,
    ValidateSecretsResponse,
};
use crate::config::SecretsConfig;
use crate::scope::ScopeContext;

const ENCRYPTION_VERSION: i32 = 1;
const NONCE_LEN: usize = 12;
const DATA_KEY_LEN: usize = 32;

pub trait SecretCipher: Send + Sync {
    fn encrypt(
        &self,
        secret_id: &str,
        plaintext: &str,
    ) -> Result<EncryptedSecretValue, SecretError>;
    fn decrypt(&self, record: &SecretRecord) -> Result<String, SecretError>;
}

/// Local envelope cipher backed by a bootstrap master key.
///
/// The data key is generated per secret and encrypted by the master key. This
/// keeps the database ciphertext-only while leaving room to replace this with
/// AWS KMS without changing the DAO/API shape.
pub struct LocalEnvelopeSecretCipher {
    master_key: [u8; DATA_KEY_LEN],
    kms_key_id: String,
}

impl LocalEnvelopeSecretCipher {
    pub fn from_config(config: &SecretsConfig) -> Option<Arc<dyn SecretCipher>> {
        let raw = config.encryption_key.as_deref()?;
        let Some(master_key) = parse_master_key(raw) else {
            tracing::warn!("SECRETS_ENCRYPTION_KEY must be base64 or raw 32 bytes");
            return None;
        };
        let kms_key_id = config
            .kms_key_id
            .clone()
            .unwrap_or_else(|| "local:SECRETS_ENCRYPTION_KEY".to_string());
        Some(Arc::new(Self {
            master_key,
            kms_key_id,
        }))
    }
}

impl SecretCipher for LocalEnvelopeSecretCipher {
    fn encrypt(
        &self,
        secret_id: &str,
        plaintext: &str,
    ) -> Result<EncryptedSecretValue, SecretError> {
        let data_key: [u8; DATA_KEY_LEN] = rand::random();
        let value_nonce: [u8; NONCE_LEN] = rand::random();
        let key_nonce: [u8; NONCE_LEN] = rand::random();

        let value_cipher =
            Aes256Gcm::new_from_slice(&data_key).map_err(|_| SecretError::EncryptionFailed)?;
        let value_ciphertext = value_cipher
            .encrypt(
                Nonce::from_slice(&value_nonce),
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: secret_id.as_bytes(),
                },
            )
            .map_err(|_| SecretError::EncryptionFailed)?;

        let key_cipher = Aes256Gcm::new_from_slice(&self.master_key)
            .map_err(|_| SecretError::EncryptionFailed)?;
        let encrypted_key_body = key_cipher
            .encrypt(
                Nonce::from_slice(&key_nonce),
                Payload {
                    msg: &data_key,
                    aad: b"moatless-secret-data-key-v1",
                },
            )
            .map_err(|_| SecretError::EncryptionFailed)?;
        let mut encrypted_data_key = Vec::with_capacity(NONCE_LEN + encrypted_key_body.len());
        encrypted_data_key.extend_from_slice(&key_nonce);
        encrypted_data_key.extend_from_slice(&encrypted_key_body);

        Ok(EncryptedSecretValue {
            value_ciphertext,
            encryption_nonce: value_nonce.to_vec(),
            encrypted_data_key,
            kms_key_id: self.kms_key_id.clone(),
            encryption_version: ENCRYPTION_VERSION,
        })
    }

    fn decrypt(&self, record: &SecretRecord) -> Result<String, SecretError> {
        if record.encryption_version != ENCRYPTION_VERSION {
            return Err(SecretError::DecryptionFailed);
        }

        let encrypted_data_key = record
            .encrypted_data_key
            .as_deref()
            .ok_or(SecretError::DecryptionFailed)?;
        if encrypted_data_key.len() <= NONCE_LEN {
            return Err(SecretError::DecryptionFailed);
        }
        let (key_nonce, encrypted_key_body) = encrypted_data_key.split_at(NONCE_LEN);

        let key_cipher = Aes256Gcm::new_from_slice(&self.master_key)
            .map_err(|_| SecretError::DecryptionFailed)?;
        let data_key = key_cipher
            .decrypt(
                Nonce::from_slice(key_nonce),
                Payload {
                    msg: encrypted_key_body,
                    aad: b"moatless-secret-data-key-v1",
                },
            )
            .map_err(|_| SecretError::DecryptionFailed)?;

        let value_cipher =
            Aes256Gcm::new_from_slice(&data_key).map_err(|_| SecretError::DecryptionFailed)?;
        let nonce = record
            .encryption_nonce
            .as_deref()
            .ok_or(SecretError::DecryptionFailed)?;
        let ciphertext = record
            .value_ciphertext
            .as_deref()
            .ok_or(SecretError::DecryptionFailed)?;
        let plaintext = value_cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: record.id.as_bytes(),
                },
            )
            .map_err(|_| SecretError::DecryptionFailed)?;

        String::from_utf8(plaintext).map_err(|_| SecretError::DecryptionFailed)
    }
}

fn parse_master_key(raw: &str) -> Option<[u8; DATA_KEY_LEN]> {
    let decoded = base64::engine::general_purpose::STANDARD.decode(raw).ok();
    let bytes = decoded
        .as_deref()
        .filter(|bytes| bytes.len() == DATA_KEY_LEN)
        .unwrap_or(raw.as_bytes());
    if bytes.len() != DATA_KEY_LEN {
        return None;
    }
    let mut key = [0_u8; DATA_KEY_LEN];
    key.copy_from_slice(bytes);
    Some(key)
}

#[derive(Clone)]
pub struct SecretService {
    secrets: Arc<dyn SecretRepository>,
    audits: Arc<dyn SecretAuditRepository>,
    cipher: Option<Arc<dyn SecretCipher>>,
}

impl SecretService {
    pub fn new(
        secrets: Arc<dyn SecretRepository>,
        audits: Arc<dyn SecretAuditRepository>,
        cipher: Option<Arc<dyn SecretCipher>>,
    ) -> Self {
        Self {
            secrets,
            audits,
            cipher,
        }
    }

    pub async fn list_metadata(
        &self,
        ctx: &ScopeContext,
        scope_type: SecretScopeType,
        scope_id: Option<&str>,
    ) -> Result<Vec<SecretMetadataResponse>, SecretError> {
        let scope_id = normalize_scope_id(scope_type, scope_id, &ctx.user_id)?;
        ensure_scope_access(ctx, scope_type, scope_id.as_deref())?;
        let records = self
            .secrets
            .list_by_scope(scope_type.as_str(), scope_id.as_deref())
            .await?;
        records
            .into_iter()
            .map(SecretMetadataResponse::try_from)
            .collect::<Result<Vec<_>, _>>()
            .map_err(SecretError::Internal)
    }

    pub async fn get_metadata(
        &self,
        ctx: &ScopeContext,
        id: &str,
    ) -> Result<SecretMetadataResponse, SecretError> {
        let record = self
            .secrets
            .find_by_id(id)
            .await?
            .ok_or_else(|| SecretError::NotFound(id.to_string()))?;
        let scope_type =
            SecretScopeType::try_from(record.scope_type.as_str()).map_err(SecretError::Internal)?;
        ensure_scope_access(ctx, scope_type, record.scope_id.as_deref())?;
        SecretMetadataResponse::try_from(record).map_err(SecretError::Internal)
    }

    pub async fn put_secret(
        &self,
        ctx: &ScopeContext,
        actor_id: &str,
        req: PutSecretRequest,
    ) -> Result<SecretMetadataResponse, SecretError> {
        let scope_id = normalize_scope_id(req.scope_type, req.scope_id.as_deref(), actor_id)?;
        ensure_scope_access(ctx, req.scope_type, scope_id.as_deref())?;
        validate_secret_shape(&req.key, req.value.as_ref(), req.external_ref.as_ref())?;

        let id = self
            .secrets
            .find_by_scope_key(req.scope_type.as_str(), scope_id.as_deref(), &req.key)
            .await?
            .map_or_else(|| uuid::Uuid::new_v4().to_string(), |existing| existing.id);

        let storage =
            self.prepare_storage(&id, req.value.as_deref(), req.external_ref.as_deref())?;
        let existing = self.secrets.find_by_id(&id).await?.map(|record| record.id);

        let record = if existing.is_some() {
            self.secrets
                .replace(SecretReplace {
                    id: &id,
                    kind: req.kind.as_str(),
                    provider: req.provider.as_deref(),
                    value_ciphertext: storage.value_ciphertext.as_deref(),
                    encryption_nonce: storage.encryption_nonce.as_deref(),
                    encrypted_data_key: storage.encrypted_data_key.as_deref(),
                    external_ref: storage.external_ref.as_deref(),
                    kms_key_id: storage.kms_key_id.as_deref(),
                    encryption_version: storage.encryption_version,
                    actor_id,
                })
                .await?
        } else {
            self.secrets
                .insert(SecretInsert {
                    id: &id,
                    scope_type: req.scope_type.as_str(),
                    scope_id: scope_id.as_deref(),
                    key: &req.key,
                    kind: req.kind.as_str(),
                    provider: req.provider.as_deref(),
                    value_ciphertext: storage.value_ciphertext.as_deref(),
                    encryption_nonce: storage.encryption_nonce.as_deref(),
                    encrypted_data_key: storage.encrypted_data_key.as_deref(),
                    external_ref: storage.external_ref.as_deref(),
                    kms_key_id: storage.kms_key_id.as_deref(),
                    encryption_version: storage.encryption_version,
                    actor_id,
                })
                .await?
        };

        self.audit(
            Some(&record.id),
            actor_id,
            if existing.is_some() {
                "update"
            } else {
                "create"
            },
            &record,
            "success",
        )
        .await?;

        SecretMetadataResponse::try_from(record).map_err(SecretError::Internal)
    }

    pub async fn patch_secret(
        &self,
        ctx: &ScopeContext,
        actor_id: &str,
        id: &str,
        req: PatchSecretRequest,
    ) -> Result<SecretMetadataResponse, SecretError> {
        let existing = self
            .secrets
            .find_by_id(id)
            .await?
            .ok_or_else(|| SecretError::NotFound(id.to_string()))?;
        let scope_type = SecretScopeType::try_from(existing.scope_type.as_str())
            .map_err(SecretError::Internal)?;
        ensure_scope_access(ctx, scope_type, existing.scope_id.as_deref())?;

        let kind = req.kind.unwrap_or(
            SecretKind::try_from(existing.kind.as_str()).map_err(SecretError::Internal)?,
        );
        let provider = req.provider.or(existing.provider.clone());
        validate_secret_shape(&existing.key, req.value.as_ref(), req.external_ref.as_ref())?;

        let storage = if req.value.is_some() || req.external_ref.is_some() {
            self.prepare_storage(
                &existing.id,
                req.value.as_deref(),
                req.external_ref.as_deref(),
            )?
        } else {
            ExistingStorage::from_record(&existing)
        };

        let record = self
            .secrets
            .replace(SecretReplace {
                id: &existing.id,
                kind: kind.as_str(),
                provider: provider.as_deref(),
                value_ciphertext: storage.value_ciphertext.as_deref(),
                encryption_nonce: storage.encryption_nonce.as_deref(),
                encrypted_data_key: storage.encrypted_data_key.as_deref(),
                external_ref: storage.external_ref.as_deref(),
                kms_key_id: storage.kms_key_id.as_deref(),
                encryption_version: storage.encryption_version,
                actor_id,
            })
            .await?;

        self.audit(Some(&record.id), actor_id, "update", &record, "success")
            .await?;
        SecretMetadataResponse::try_from(record).map_err(SecretError::Internal)
    }

    pub async fn delete_secret(
        &self,
        ctx: &ScopeContext,
        actor_id: &str,
        id: &str,
    ) -> Result<(), SecretError> {
        let record = self
            .secrets
            .find_by_id(id)
            .await?
            .ok_or_else(|| SecretError::NotFound(id.to_string()))?;
        let scope_type =
            SecretScopeType::try_from(record.scope_type.as_str()).map_err(SecretError::Internal)?;
        ensure_scope_access(ctx, scope_type, record.scope_id.as_deref())?;
        if !self.secrets.delete(id).await? {
            return Err(SecretError::NotFound(id.to_string()));
        }
        self.audit(Some(id), actor_id, "delete", &record, "success")
            .await
    }

    pub async fn validate_required(
        &self,
        ctx: &ScopeContext,
        req: ValidateSecretsRequest,
    ) -> Result<ValidateSecretsResponse, SecretError> {
        let scope_id = normalize_scope_id(req.scope_type, req.scope_id.as_deref(), &ctx.user_id)?;
        ensure_scope_access(ctx, req.scope_type, scope_id.as_deref())?;
        let records = self
            .secrets
            .list_by_scope(req.scope_type.as_str(), scope_id.as_deref())
            .await?;
        let required: HashSet<String> = req.required.into_iter().collect();
        let present: HashSet<String> = records
            .into_iter()
            .map(|record| record.key)
            .filter(|key| required.contains(key))
            .collect();
        let missing = required
            .difference(&present)
            .cloned()
            .collect::<Vec<String>>();
        Ok(ValidateSecretsResponse {
            present: present.into_iter().collect(),
            missing,
        })
    }

    /// Resolve plaintext secrets for an already-authorized internal workflow.
    pub async fn resolve_plaintext_scope(
        &self,
        actor_id: &str,
        scope_type: SecretScopeType,
        scope_id: Option<&str>,
    ) -> Result<Vec<ResolvedSecret>, SecretError> {
        let scope_id = normalize_scope_id(scope_type, scope_id, actor_id)?;
        let records = self
            .secrets
            .list_by_scope(scope_type.as_str(), scope_id.as_deref())
            .await?;
        let mut resolved = Vec::new();
        for record in records {
            if is_reserved_key(&record.key) {
                continue;
            }
            let value = match self.decrypt_record(&record) {
                Ok(value) => value,
                Err(SecretError::ExternalRefUnsupported(ref external_ref)) => {
                    tracing::warn!(
                        secret_id = %record.id,
                        external_ref = %external_ref,
                        "Skipping unsupported external secret reference"
                    );
                    continue;
                }
                Err(e) => return Err(e),
            };
            self.secrets.mark_used(&record.id).await?;
            self.audit(Some(&record.id), actor_id, "resolve", &record, "success")
                .await?;
            resolved.push(ResolvedSecret {
                key: record.key,
                value,
                provider: record.provider,
                kind: record.kind,
            });
        }
        Ok(resolved)
    }

    /// Resolve accessible secrets into an env-var map with `user > team >
    /// global` precedence.
    ///
    /// `ssh_key`-kind secrets are intentionally excluded — they are delivered
    /// through a separate channel and materialized into `~/.ssh` rather than
    /// being exposed as environment variables. Use
    /// [`resolve_accessible_ssh_keys`](Self::resolve_accessible_ssh_keys) for those.
    pub async fn resolve_accessible_plaintext(
        &self,
        actor_id: &str,
        team_id: Option<&str>,
    ) -> Result<HashMap<String, String>, SecretError> {
        self.resolve_accessible_by_kind(actor_id, team_id, false)
            .await
    }

    /// Resolve accessible `ssh_key`-kind secrets into a name → private-key map
    /// with `user > team > global` precedence. Delivered to the sandbox
    /// separately from env credentials and written to `~/.ssh`.
    pub async fn resolve_accessible_ssh_keys(
        &self,
        actor_id: &str,
        team_id: Option<&str>,
    ) -> Result<HashMap<String, String>, SecretError> {
        self.resolve_accessible_by_kind(actor_id, team_id, true)
            .await
    }

    /// Shared implementation for the two accessible-secret resolvers.
    /// When `ssh_keys_only` is true, only `ssh_key`-kind secrets are kept;
    /// otherwise `ssh_key`-kind secrets are skipped.
    async fn resolve_accessible_by_kind(
        &self,
        actor_id: &str,
        team_id: Option<&str>,
        ssh_keys_only: bool,
    ) -> Result<HashMap<String, String>, SecretError> {
        let ssh_kind = SecretKind::SshKey.as_str();
        let keep = |scope: &ResolvedSecret| (scope.kind.as_str() == ssh_kind) == ssh_keys_only;

        let mut credentials = HashMap::new();
        for scope in self
            .resolve_plaintext_scope(actor_id, SecretScopeType::Global, None)
            .await?
        {
            if keep(&scope) {
                credentials.insert(scope.key, scope.value);
            }
        }
        if let Some(team_id) = team_id {
            for scope in self
                .resolve_plaintext_scope(actor_id, SecretScopeType::Team, Some(team_id))
                .await?
            {
                if keep(&scope) {
                    credentials.entry(scope.key).or_insert(scope.value);
                }
            }
        }
        for scope in self
            .resolve_plaintext_scope(actor_id, SecretScopeType::User, Some(actor_id))
            .await?
        {
            if keep(&scope) {
                credentials.entry(scope.key).or_insert(scope.value);
            }
        }
        Ok(credentials)
    }

    fn prepare_storage(
        &self,
        id: &str,
        value: Option<&str>,
        external_ref: Option<&str>,
    ) -> Result<ExistingStorage, SecretError> {
        match (value, external_ref) {
            (Some(_), Some(_)) => Err(SecretError::BadRequest(
                "Only one of value or externalRef may be set".to_string(),
            )),
            (Some(value), None) => {
                let cipher = self
                    .cipher
                    .as_ref()
                    .ok_or(SecretError::EncryptionNotConfigured)?;
                let encrypted = cipher.encrypt(id, value)?;
                Ok(ExistingStorage {
                    value_ciphertext: Some(encrypted.value_ciphertext),
                    encryption_nonce: Some(encrypted.encryption_nonce),
                    encrypted_data_key: Some(encrypted.encrypted_data_key),
                    external_ref: None,
                    kms_key_id: Some(encrypted.kms_key_id),
                    encryption_version: encrypted.encryption_version,
                })
            }
            (None, Some(external_ref)) if !external_ref.trim().is_empty() => Ok(ExistingStorage {
                value_ciphertext: None,
                encryption_nonce: None,
                encrypted_data_key: None,
                external_ref: Some(external_ref.to_string()),
                kms_key_id: None,
                encryption_version: ENCRYPTION_VERSION,
            }),
            (None, Some(_)) => Err(SecretError::BadRequest(
                "externalRef cannot be empty".to_string(),
            )),
            (None, None) => Err(SecretError::BadRequest(
                "A secret value or externalRef is required".to_string(),
            )),
        }
    }

    fn decrypt_record(&self, record: &SecretRecord) -> Result<String, SecretError> {
        if let Some(ref external_ref) = record.external_ref {
            return Err(SecretError::ExternalRefUnsupported(external_ref.clone()));
        }
        let cipher = self
            .cipher
            .as_ref()
            .ok_or(SecretError::EncryptionNotConfigured)?;
        cipher.decrypt(record)
    }

    async fn audit(
        &self,
        secret_id: Option<&str>,
        actor_id: &str,
        action: &str,
        record: &SecretRecord,
        result: &str,
    ) -> Result<(), SecretError> {
        self.audits
            .record(SecretAuditInput {
                secret_id,
                actor_id,
                action,
                scope_type: &record.scope_type,
                scope_id: record.scope_id.as_deref(),
                key: &record.key,
                result,
            })
            .await?;
        Ok(())
    }
}

struct ExistingStorage {
    value_ciphertext: Option<Vec<u8>>,
    encryption_nonce: Option<Vec<u8>>,
    encrypted_data_key: Option<Vec<u8>>,
    external_ref: Option<String>,
    kms_key_id: Option<String>,
    encryption_version: i32,
}

impl ExistingStorage {
    fn from_record(record: &SecretRecord) -> Self {
        Self {
            value_ciphertext: record.value_ciphertext.clone(),
            encryption_nonce: record.encryption_nonce.clone(),
            encrypted_data_key: record.encrypted_data_key.clone(),
            external_ref: record.external_ref.clone(),
            kms_key_id: record.kms_key_id.clone(),
            encryption_version: record.encryption_version,
        }
    }
}

fn normalize_scope_id(
    scope_type: SecretScopeType,
    scope_id: Option<&str>,
    actor_id: &str,
) -> Result<Option<String>, SecretError> {
    match scope_type {
        SecretScopeType::Global => {
            if scope_id.is_some() {
                return Err(SecretError::BadRequest(
                    "global secrets must not set scopeId".to_string(),
                ));
            }
            Ok(None)
        }
        SecretScopeType::Team => scope_id
            .filter(|id| !id.trim().is_empty())
            .map(std::string::ToString::to_string)
            .map(Some)
            .ok_or_else(|| SecretError::BadRequest("team secrets require scopeId".to_string())),
        SecretScopeType::User => Ok(Some(
            scope_id
                .filter(|id| !id.trim().is_empty())
                .unwrap_or(actor_id)
                .to_string(),
        )),
    }
}

fn ensure_scope_access(
    ctx: &ScopeContext,
    scope_type: SecretScopeType,
    scope_id: Option<&str>,
) -> Result<(), SecretError> {
    if ctx.is_admin {
        return Ok(());
    }

    match scope_type {
        SecretScopeType::Global => Err(SecretError::Forbidden(
            "global secrets require admin access".to_string(),
        )),
        SecretScopeType::Team => {
            let Some(team_id) = scope_id else {
                return Err(SecretError::BadRequest(
                    "team secrets require scopeId".to_string(),
                ));
            };
            if ctx.team_ids.iter().any(|id| id == team_id) {
                Ok(())
            } else {
                Err(SecretError::Forbidden(
                    "team secret is outside this user's teams".to_string(),
                ))
            }
        }
        SecretScopeType::User => {
            let Some(user_id) = scope_id else {
                return Err(SecretError::BadRequest(
                    "user secrets require scopeId".to_string(),
                ));
            };
            if user_id == ctx.user_id {
                Ok(())
            } else {
                Err(SecretError::Forbidden(
                    "user secret is outside this user".to_string(),
                ))
            }
        }
    }
}

fn validate_secret_shape(
    key: &str,
    value: Option<&String>,
    external_ref: Option<&String>,
) -> Result<(), SecretError> {
    if !is_valid_secret_key(key) {
        return Err(SecretError::BadRequest(format!(
            "secret key must match ^[A-Z_][A-Z0-9_]*$: {key}"
        )));
    }
    if is_reserved_key(key) {
        return Err(SecretError::BadRequest(format!(
            "reserved runtime key cannot be used as a sandbox secret: {key}"
        )));
    }
    if value.is_some() && external_ref.is_some() {
        return Err(SecretError::BadRequest(
            "Only one of value or externalRef may be set".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_envelope_cipher_round_trips_without_plaintext_metadata() {
        let config = SecretsConfig {
            encryption_key: Some("0123456789abcdef0123456789abcdef".to_string()),
            kms_key_id: None,
        };
        let cipher = LocalEnvelopeSecretCipher::from_config(&config).unwrap();
        let encrypted = cipher.encrypt("secret-id", "super-secret").unwrap();
        assert_ne!(encrypted.value_ciphertext, b"super-secret");
        let record = SecretRecord {
            id: "secret-id".to_string(),
            scope_type: "user".to_string(),
            scope_id: Some("u1".to_string()),
            key: "TOKEN".to_string(),
            kind: "env".to_string(),
            provider: None,
            value_ciphertext: Some(encrypted.value_ciphertext),
            encryption_nonce: Some(encrypted.encryption_nonce),
            encrypted_data_key: Some(encrypted.encrypted_data_key),
            external_ref: None,
            kms_key_id: Some(encrypted.kms_key_id),
            encryption_version: encrypted.encryption_version,
            created_by: "u1".to_string(),
            updated_by: "u1".to_string(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_used_at: None,
            version: 1,
        };
        assert_eq!(cipher.decrypt(&record).unwrap(), "super-secret");
    }

    #[test]
    fn normalizes_user_scope_to_actor() {
        assert_eq!(
            normalize_scope_id(SecretScopeType::User, None, "u1").unwrap(),
            Some("u1".to_string())
        );
    }

    #[test]
    fn non_admin_cannot_create_global_secret() {
        let ctx = ScopeContext {
            user_id: "u1".to_string(),
            team_ids: vec![],
            is_admin: false,
        };
        assert!(ensure_scope_access(&ctx, SecretScopeType::Global, None).is_err());
    }
}
