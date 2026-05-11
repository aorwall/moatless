use axum::extract::{Query, State};
use axum::Json;
use chrono::Utc;
use moatless_api_schemas::adapters::{AdapterConnectionResponse, CreateAdapterConnectionRequest};

use super::common::{
    adapter_connection_response, ensure_can_write_adapter_resource,
    resolve_scope_for_adapter_create, scoped_resource, AdapterFilter,
};
use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::error::AdapterError;
use crate::adapters::models::AdapterConnection;
use crate::adapters::AdapterKind;
use crate::extractors::AuthUser;
use crate::state::AppState;

#[utoipa::path(
    get,
    path = "/api/v1/adapters",
    tag = "adapters",
    responses((status = 200, description = "Configured adapter names", body = Vec<String>))
)]
pub async fn list_adapters(State(state): State<AppState>) -> Json<Vec<String>> {
    let types = state
        .adapters
        .as_ref()
        .map(|p| p.registered_types())
        .unwrap_or_default();

    let adapters = types
        .into_iter()
        .map(|t| match t {
            AdapterKind::Slack => "slack".to_string(),
            AdapterKind::Telegram => "telegram".to_string(),
            AdapterKind::Github => "github".to_string(),
            AdapterKind::GithubPr => "github".to_string(),
            AdapterKind::Linear => "linear".to_string(),
            AdapterKind::Webhook => "webhook".to_string(),
        })
        .collect();

    Json(adapters)
}

#[utoipa::path(
    get,
    path = "/api/v1/adapters/connections",
    tag = "adapters",
    responses((status = 200, description = "Adapter connections", body = Vec<AdapterConnectionResponse>))
)]
pub async fn list_adapter_connections(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<AdapterFilter>,
) -> Result<Json<Vec<AdapterConnectionResponse>>, AdapterError> {
    let ctx = auth.scope_context();
    let provider = query.adapter_kind.map(|p| p.as_str().to_string());
    let rows = PgAdapterConnectionRepository(state.db.clone())
        .list(&ctx, provider.as_deref())
        .await?;
    Ok(Json(rows.iter().map(adapter_connection_response).collect()))
}

#[utoipa::path(
    post,
    path = "/api/v1/adapters/connections",
    tag = "adapters",
    request_body = CreateAdapterConnectionRequest,
    responses((status = 200, description = "Adapter connection upserted", body = AdapterConnectionResponse))
)]
pub async fn create_adapter_connection(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<CreateAdapterConnectionRequest>,
) -> Result<Json<AdapterConnectionResponse>, AdapterError> {
    if req.external_account_id.trim().is_empty() {
        return Err(AdapterError::BadRequest(
            "external_account_id is required".into(),
        ));
    }

    let now = Utc::now();
    let provider = req.adapter_kind.as_str().to_string();
    let repo = PgAdapterConnectionRepository(state.db.clone());
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
        ensure_can_write_adapter_resource(&state, &auth, &resource, "adapter connection").await?;
    }
    let resolved =
        resolve_scope_for_adapter_create(&state, &auth, req.scope, req.team_id.clone()).await?;
    let row = AdapterConnection {
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
    Ok(Json(adapter_connection_response(&saved)))
}
