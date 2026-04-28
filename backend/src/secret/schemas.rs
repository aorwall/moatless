//! Secret API request and response DTOs.

use std::collections::HashMap;

#[cfg(test)]
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::models::{SecretKind, SecretRecord, SecretScopeType, SecretSource};

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretMetadataResponse {
    pub id: String,
    pub scope_type: SecretScopeType,
    pub scope_id: Option<String>,
    pub key: String,
    pub kind: SecretKind,
    pub provider: Option<String>,
    pub source: SecretSource,
    pub version: i32,
    pub updated_at: String,
    pub last_used_at: Option<String>,
}

impl TryFrom<SecretRecord> for SecretMetadataResponse {
    type Error = String;

    fn try_from(record: SecretRecord) -> Result<Self, Self::Error> {
        let source = record.source();
        Ok(Self {
            id: record.id,
            scope_type: SecretScopeType::try_from(record.scope_type.as_str())?,
            scope_id: record.scope_id,
            key: record.key,
            kind: SecretKind::try_from(record.kind.as_str())?,
            provider: record.provider,
            source,
            version: record.version,
            updated_at: record.updated_at.to_rfc3339(),
            last_used_at: record.last_used_at.map(|dt| dt.to_rfc3339()),
        })
    }
}

#[derive(Debug, Clone, Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(rename_all = "camelCase")]
pub struct ListSecretsQuery {
    #[serde(rename = "scope")]
    pub scope_type: SecretScopeType,
    pub scope_id: Option<String>,
}

#[derive(Clone, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PutSecretRequest {
    #[serde(rename = "scope")]
    pub scope_type: SecretScopeType,
    pub scope_id: Option<String>,
    pub key: String,
    pub kind: SecretKind,
    pub provider: Option<String>,
    pub value: Option<String>,
    pub external_ref: Option<String>,
}

impl std::fmt::Debug for PutSecretRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PutSecretRequest")
            .field("scope_type", &self.scope_type)
            .field("scope_id", &self.scope_id)
            .field("key", &self.key)
            .field("kind", &self.kind)
            .field("provider", &self.provider)
            .field("value", &self.value.as_ref().map(|_| "[REDACTED]"))
            .field("external_ref", &self.external_ref)
            .finish()
    }
}

#[derive(Clone, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PatchSecretRequest {
    pub kind: Option<SecretKind>,
    pub provider: Option<String>,
    pub value: Option<String>,
    pub external_ref: Option<String>,
}

impl std::fmt::Debug for PatchSecretRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PatchSecretRequest")
            .field("kind", &self.kind)
            .field("provider", &self.provider)
            .field("value", &self.value.as_ref().map(|_| "[REDACTED]"))
            .field("external_ref", &self.external_ref)
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidateSecretsRequest {
    #[serde(rename = "scope")]
    pub scope_type: SecretScopeType,
    pub scope_id: Option<String>,
    pub required: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValidateSecretsResponse {
    pub present: Vec<String>,
    pub missing: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolveSecretsRequest {
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResolveSecretsResponse {
    pub credentials: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretMutationResponse {
    pub secret: SecretMetadataResponse,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn put_request_debug_redacts_secret_value() {
        let req = PutSecretRequest {
            scope_type: SecretScopeType::User,
            scope_id: Some("repo_1".to_string()),
            key: "STRIPE_SECRET_KEY".to_string(),
            kind: SecretKind::Env,
            provider: None,
            value: Some("sk_live_secret".to_string()),
            external_ref: None,
        };

        let debug = format!("{req:?}");
        assert!(!debug.contains("sk_live_secret"));
        assert!(debug.contains("[REDACTED]"));
    }

    #[test]
    fn metadata_response_serializes_without_value_field() {
        let resp = SecretMetadataResponse {
            id: "id".to_string(),
            scope_type: SecretScopeType::User,
            scope_id: Some("repo".to_string()),
            key: "STRIPE_SECRET_KEY".to_string(),
            kind: SecretKind::Env,
            provider: None,
            source: SecretSource::InlineEncrypted,
            version: 1,
            updated_at: Utc::now().to_rfc3339(),
            last_used_at: None,
        };

        let json = serde_json::to_value(resp).unwrap();
        assert!(json.get("value").is_none());
        assert!(json.get("valueCiphertext").is_none());
        assert!(json.get("externalRef").is_none());
    }
}
