use axum::extract::{Query, State};
use axum::Json;
use chrono::Utc;
use moatless_api_schemas::channel::{CreateProviderConnectionRequest, ProviderConnectionResponse};

use super::common::{
    ensure_can_write_channel_resource, provider_connection_response,
    resolve_scope_for_channel_create, scoped_resource, ProviderFilter,
};
use crate::channel::dao::connections::PgProviderConnectionRepository;
use crate::channel::error::ChannelError;
use crate::channel::models::ProviderConnection;
use crate::channel::provider::ChannelType;
use crate::extractors::AuthUser;
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/api/v1/integrations/providers",
    tag = "integrations",
    responses((status = 200, description = "Configured provider names", body = Vec<String>))
)]
pub async fn list_integration_providers(State(state): State<AppState>) -> Json<Vec<String>> {
    let types = state
        .channel_providers
        .as_ref()
        .map(|p| p.registered_types())
        .unwrap_or_default();

    let providers = types
        .into_iter()
        .filter_map(|t| match t {
            ChannelType::Slack => Some("slack".to_string()),
            ChannelType::Telegram => Some("telegram".to_string()),
            ChannelType::GithubPr => Some("github".to_string()),
            ChannelType::Linear => Some("linear".to_string()),
            ChannelType::Webhook => Some("webhook".to_string()),
            ChannelType::Email | ChannelType::Cron => None,
        })
        .collect();

    Json(providers)
}

#[utoipa::path(
    get,
    path = "/api/v1/integrations/connections",
    tag = "integrations",
    responses((status = 200, description = "Provider connections", body = Vec<ProviderConnectionResponse>))
)]
pub async fn list_provider_connections(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ProviderFilter>,
) -> Result<Json<Vec<ProviderConnectionResponse>>, ChannelError> {
    let ctx = auth.scope_context();
    let provider = query.provider.map(|p| p.as_str().to_string());
    let rows = PgProviderConnectionRepository(state.db.clone())
        .list(&ctx, provider.as_deref())
        .await?;
    Ok(Json(
        rows.iter().map(provider_connection_response).collect(),
    ))
}

#[utoipa::path(
    post,
    path = "/api/v1/integrations/connections",
    tag = "integrations",
    request_body = CreateProviderConnectionRequest,
    responses((status = 200, description = "Provider connection upserted", body = ProviderConnectionResponse))
)]
pub async fn create_provider_connection(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateProviderConnectionRequest>,
) -> Result<Json<ProviderConnectionResponse>, ChannelError> {
    if req.external_account_id.trim().is_empty() {
        return Err(ChannelError::BadRequest(
            "external_account_id is required".into(),
        ));
    }

    let now = Utc::now();
    let provider = req.provider.as_str().to_string();
    let repo = PgProviderConnectionRepository(state.db.clone());
    if let Some(existing) = repo
        .find_by_external_account(&provider, &req.external_account_id)
        .await?
    {
        let resource = scoped_resource(
            existing.scope,
            existing.team_id.as_deref(),
            existing.owner_user_id.as_deref(),
            &existing.created_by,
        );
        ensure_can_write_channel_resource(&state, &auth, &resource, "provider connection").await?;
    }
    let resolved =
        resolve_scope_for_channel_create(&state, &auth, req.scope, req.team_id.clone()).await?;
    let row = ProviderConnection {
        id: format!("pc_{}", uuid::Uuid::new_v4()),
        provider,
        connection_kind: req.connection_kind.unwrap_or_else(|| "webhook".to_string()),
        scope_value: req.external_account_id,
        credential_ref: None,
        webhook_secret: req.webhook_secret,
        transport_config: req
            .transport_config
            .unwrap_or_else(|| serde_json::json!({})),
        created_by: auth.user_id,
        scope: resolved.scope,
        team_id: Some(resolved.team_id),
        owner_user_id: resolved.owner_user_id,
        created_at: now,
        updated_at: now,
    };
    let saved = repo.upsert(&row).await?;
    Ok(Json(provider_connection_response(&saved)))
}
