//! Secret API request and response DTOs.

#[cfg(test)]
use chrono::Utc;
use serde::{Deserialize, Serialize};

use super::models::SecretRecord;
use moatless_api_schemas::enums::Scope;
use moatless_types::secret::{SecretKind, SecretSource};

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretMetadataResponse {
    pub id: String,
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    pub key: String,
    pub kind: SecretKind,
    pub provider: Option<String>,
    pub source: SecretSource,
    pub enabled: bool,
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
            scope: Scope::try_from(record.scope.as_str())?,
            owner_user_id: record.owner_user_id,
            key: record.key,
            kind: SecretKind::try_from(record.kind.as_str())?,
            provider: record.provider,
            source,
            enabled: record.enabled,
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
    pub scope: Option<Scope>,
}

#[derive(Clone, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PutSecretRequest {
    pub scope: Option<Scope>,
    pub key: String,
    pub kind: SecretKind,
    pub provider: Option<String>,
    pub value: Option<String>,
    pub external_ref: Option<String>,
}

impl std::fmt::Debug for PutSecretRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PutSecretRequest")
            .field("scope", &self.scope)
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
    /// Toggle the secret active/inactive without touching its value.
    pub enabled: Option<bool>,
}

impl std::fmt::Debug for PatchSecretRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PatchSecretRequest")
            .field("kind", &self.kind)
            .field("provider", &self.provider)
            .field("value", &self.value.as_ref().map(|_| "[REDACTED]"))
            .field("external_ref", &self.external_ref)
            .field("enabled", &self.enabled)
            .finish()
    }
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
            scope: Some(Scope::User),
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
        use moatless_types::secret::SecretSource;
        let resp = SecretMetadataResponse {
            id: "id".to_string(),
            scope: Scope::User,
            owner_user_id: Some("u1".to_string()),
            key: "STRIPE_SECRET_KEY".to_string(),
            kind: SecretKind::Env,
            provider: None,
            source: SecretSource::InlineEncrypted,
            enabled: true,
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
