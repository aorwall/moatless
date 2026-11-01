//! Secret request API types.

use serde::{Deserialize, Serialize};

use moatless_types::access_control::Scope;
use moatless_types::secret::SecretKind;

#[derive(Debug, Clone, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateSecretRequestRequest {
    pub key: String,
    pub scope: Scope,
    pub kind: SecretKind,
    pub label: Option<String>,
    #[serde(default)]
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecretRequestResponse {
    pub request_id: String,
    pub key: String,
    pub scope: Scope,
    pub kind: SecretKind,
    pub label: String,
}
