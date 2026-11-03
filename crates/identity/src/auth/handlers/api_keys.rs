//! API key and pairing code management.

use super::session::authenticate_from_cookie;
use crate::auth::api_key;
use crate::auth::error::AuthError;
use crate::auth::schemas::{
    ApiKeyCreateResponse, ApiKeyListItem, CreateApiKeyRequest, CreatePairingCodeRequest,
    MessageResponse, PairingCodeResponse, format_datetime, format_datetime_opt,
};
use crate::auth::state::AuthState;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use moatless_api_schemas::error::ErrorBody;

/// Create a new API key for the authenticated user.
#[utoipa::path(
    post,
    path = "/api/v1/auth/api-keys",
    tag = "api-keys",
    request_body = CreateApiKeyRequest,
    responses(
        (status = 201, description = "API key created", body = ApiKeyCreateResponse),
        (status = 401, description = "Not authenticated", body = ErrorBody)
    )
)]
pub(in crate::auth) async fn create_api_key_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(body): Json<CreateApiKeyRequest>,
) -> Result<(StatusCode, Json<ApiKeyCreateResponse>), AuthError> {
    let claims = authenticate_from_cookie(&state, &headers).await?;

    let expires_at = body
        .expires_in_days
        .map(|days| chrono::Utc::now() + chrono::Duration::days(days as i64));

    let result = api_key::generate_key(
        state.api_keys.as_ref(),
        &claims.sub.to_string(),
        &body.name,
        body.scopes,
        expires_at,
        claims.group_id.as_deref(),
        None,
    )
    .await?;

    Ok((
        StatusCode::CREATED,
        Json(ApiKeyCreateResponse {
            id: result.id,
            key: result.key,
            name: result.name,
            key_prefix: result.key_prefix,
            scopes: result.scopes,
            expires_at: format_datetime_opt(result.expires_at),
            created_at: format_datetime(result.created_at),
        }),
    ))
}

/// Mint a one-time code that pairs a client on another device to this account.
///
/// The client exchanges it once, at `POST /oauth/token`, for the API key it
/// keeps; readable in this response and nowhere afterwards.
#[utoipa::path(
    post,
    path = "/api/v1/auth/pairing-codes",
    tag = "auth",
    request_body = CreatePairingCodeRequest,
    responses(
        (status = 201, description = "Pairing code created", body = PairingCodeResponse),
        (status = 401, description = "Not authenticated", body = ErrorBody)
    )
)]
pub(in crate::auth) async fn create_pairing_code_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Json(body): Json<CreatePairingCodeRequest>,
) -> Result<(StatusCode, Json<PairingCodeResponse>), AuthError> {
    // Cookie only, deliberately. A pairing code is a credential that mints
    // another credential, so the one thing that may ask for it is a person at a
    // browser they logged into — not a key, and least of all a key some other
    // device was paired with.
    let claims = authenticate_from_cookie(&state, &headers).await?;

    let issued = state
        .pairing
        .issue_code(
            &claims.sub.to_string(),
            claims.group_id.as_deref(),
            body.label.as_deref(),
        )
        .await
        .map_err(|error| AuthError::Internal(error.to_string()))?;

    Ok((
        StatusCode::CREATED,
        Json(PairingCodeResponse {
            id: issued.id,
            code: issued.code,
            label: issued.label,
            expires_at: format_datetime(issued.expires_at),
        }),
    ))
}

/// List all API keys for the authenticated user.
#[utoipa::path(
    get,
    path = "/api/v1/auth/api-keys",
    tag = "api-keys",
    responses(
        (status = 200, description = "List of API keys", body = Vec<ApiKeyListItem>),
        (status = 401, description = "Not authenticated", body = ErrorBody)
    )
)]
pub(in crate::auth) async fn list_api_keys_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
) -> Result<Json<Vec<ApiKeyListItem>>, AuthError> {
    let claims = authenticate_from_cookie(&state, &headers).await?;
    let keys = api_key::list_keys(state.api_keys.as_ref(), &claims.sub.to_string()).await?;

    let items: Vec<ApiKeyListItem> = keys
        .iter()
        .map(|k| ApiKeyListItem {
            id: k.id.clone(),
            name: k.name.clone(),
            key_prefix: k.key_prefix.clone(),
            scopes: k
                .scopes
                .as_ref()
                .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok()),
            last_used_at: format_datetime_opt(k.last_used_at),
            expires_at: format_datetime_opt(k.expires_at),
            created_at: format_datetime(k.created_at),
        })
        .collect();

    Ok(Json(items))
}

/// Revoke an API key by ID.
#[utoipa::path(
    delete,
    path = "/api/v1/auth/api-keys/{key_id}",
    tag = "api-keys",
    params(
        ("key_id" = String, Path, description = "API key identifier")
    ),
    responses(
        (status = 200, description = "API key revoked", body = MessageResponse),
        (status = 401, description = "Not authenticated", body = ErrorBody),
        (status = 404, description = "API key not found", body = ErrorBody)
    )
)]
pub(in crate::auth) async fn revoke_api_key_handler(
    State(state): State<AuthState>,
    headers: HeaderMap,
    Path(key_id): Path<String>,
) -> Result<Json<MessageResponse>, AuthError> {
    let claims = authenticate_from_cookie(&state, &headers).await?;
    let result =
        api_key::revoke_key(state.api_keys.as_ref(), &key_id, &claims.sub.to_string()).await?;

    match result {
        Some(key_hash) => {
            state.cache.invalidate_api_key(&key_hash).await;
            Ok(Json(MessageResponse {
                message: "API key revoked successfully".to_string(),
            }))
        }
        None => Err(AuthError::NotFound(
            "API key not found or not owned by user".to_string(),
        )),
    }
}
