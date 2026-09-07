use moatless_types::ui_rpc::{UiRpcAuthIdentity, UserRole};

pub use moatless_identity::auth::jwt::{SessionClaims as UiRpcSessionClaims, WS_TICKET_TTL_SECS};

pub fn generate_ws_ticket_jwt(
    identity: &UiRpcAuthIdentity,
    secret: &str,
) -> Result<String, jsonwebtoken::errors::Error> {
    let identity = moatless_types::auth::AuthIdentity {
        user_id: identity.user_id,
        login: identity.login.clone(),
        role: match identity.role {
            UserRole::Admin => moatless_types::user::UserRole::Admin,
            UserRole::User => moatless_types::user::UserRole::User,
        },
        provider: identity.provider.clone(),
        group_id: identity.group_id.clone(),
    };
    moatless_identity::auth::jwt::generate_ws_ticket_jwt(&identity, secret)
}

#[cfg(test)]
pub use moatless_identity::auth::jwt::verify_ws_ticket_jwt;
