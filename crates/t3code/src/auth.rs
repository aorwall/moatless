use moatless_types::ui_rpc::UiRpcAuthIdentity;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const WS_TICKET_TTL_SECS: u64 = 60;
const WS_TICKET_JWT_ISSUER: &str = "moatless-vibe-backend/ws-ticket";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiRpcSessionClaims {
    pub sub: Uuid,
    pub login: String,
    pub role: String,
    pub provider: String,
    pub group_id: Option<String>,
    pub iat: u64,
    pub exp: u64,
    pub iss: String,
}

pub fn generate_ws_ticket_jwt(
    identity: &UiRpcAuthIdentity,
    secret: &str,
) -> Result<String, jsonwebtoken::errors::Error> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let claims = UiRpcSessionClaims {
        sub: identity.user_id,
        login: identity.login.clone(),
        role: identity.role.as_str().to_string(),
        provider: identity.provider.clone(),
        group_id: identity.group_id.clone(),
        iat: now,
        exp: now + WS_TICKET_TTL_SECS,
        iss: WS_TICKET_JWT_ISSUER.to_string(),
    };
    jsonwebtoken::encode(
        &jsonwebtoken::Header::default(),
        &claims,
        &jsonwebtoken::EncodingKey::from_secret(secret.as_bytes()),
    )
}

#[cfg(test)]
pub fn verify_ws_ticket_jwt(
    token: &str,
    secret: &str,
) -> Result<UiRpcSessionClaims, jsonwebtoken::errors::Error> {
    let mut validation = jsonwebtoken::Validation::default();
    validation.set_issuer(&[WS_TICKET_JWT_ISSUER]);
    jsonwebtoken::decode::<UiRpcSessionClaims>(
        token,
        &jsonwebtoken::DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
}
