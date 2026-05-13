//! Linear webhook endpoint dispatcher.
//!
//! Verifies HMAC + dedup, parses the envelope, and routes to:
//! - `agent_runtime::handle_session_created` / `handle_session_prompted`
//!   for `AgentSessionEvent` payloads.
//! - The legacy `runtime.rs` path for `Issue` / `Comment` payloads (still in
//!   place until Step 4 of the refactor migrates them to `agent_runtime`).

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use tracing::{debug, info, warn};

use crate::adapters::dao::PgAdapterConnectionRepository;
use crate::adapters::error::AdapterError;
use crate::adapters::metrics::record_webhook;
use crate::adapters::AdapterKind;
use crate::state::AppState;

use super::adapter::LinearAdapter;
use super::agent_api::{default_http_client, LinearAgentApi};
use super::agent_runtime;
use super::webhook_payloads::LinearWebhookEnvelope;

#[utoipa::path(
    post,
    path = "/api/v1/adapters/linear/webhook",
    tag = "adapters",
    request_body(content = String),
    responses((status = 200, description = "Webhook processed"))
)]
pub async fn linear_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, AdapterError> {
    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("linear".into()))?;

    let linear = providers
        .get(AdapterKind::Linear)
        .ok_or_else(|| AdapterError::AdapterNotConfigured("linear".into()))?;

    let linear_provider = linear
        .as_any()
        .downcast_ref::<LinearAdapter>()
        .ok_or_else(|| AdapterError::Internal("Provider type mismatch".into()))?;

    match dispatch_webhook(&state, linear_provider, &headers, &body).await {
        Ok(()) => {
            record_webhook("linear", "success", "", "", "");
        }
        Err(e) => {
            record_webhook("linear", "error", "delivery_failed", "", "");
            warn!(error = %e, "Linear webhook processing error");
        }
    }

    Ok(StatusCode::OK)
}

/// Verify the webhook, parse the envelope, and route based on `type`.
///
/// Always returns `Ok(())` so the caller can ack quickly even on benign
/// rejections (unknown org, dedup hit, unsupported event).
async fn dispatch_webhook(
    state: &AppState,
    provider: &LinearAdapter,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<(), AdapterError> {
    let signature = headers
        .get("linear-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if signature.is_empty() {
        debug!("Linear webhook missing Linear-Signature header — ignoring");
        return Ok(());
    }

    let envelope: LinearWebhookEnvelope = serde_json::from_slice(body).map_err(|e| {
        AdapterError::BadRequest(format!("Invalid Linear webhook payload: {e}"))
    })?;

    let Some(org_id) = envelope.organization_id.as_deref() else {
        debug!("Linear webhook missing organizationId — ignoring");
        return Ok(());
    };

    let connection = PgAdapterConnectionRepository(state.db.clone())
        .find_by_external_account("linear", org_id)
        .await?;
    let Some(connection) = connection else {
        info!(org_id, "Linear webhook ignored: no integration_connection registered");
        return Ok(());
    };

    let secret = connection
        .webhook_secret
        .as_deref()
        .or(provider.global_webhook_secret_for_dispatch());
    if let Some(secret) = secret {
        super::signature::verify_signature(secret, body, signature)?;
    } else {
        warn!(
            org_id,
            "Linear webhook arrived but no webhook secret configured on connection or env — refusing"
        );
        return Err(AdapterError::SignatureInvalid);
    }

    if let Some(ref webhook_id) = envelope.webhook_id {
        if !provider.mark_seen_for_dispatch(webhook_id).await {
            debug!(webhook_id, "Linear webhook deduplicated");
            return Ok(());
        }
    }

    let event_type = envelope.event_type.as_str();
    match event_type {
        "AgentSessionEvent" => {
            let api = build_agent_api(state, &connection.scope_value);
            handle_agent_session_event(state, &api, &connection, &envelope).await
        }
        "Issue" => {
            let api = build_agent_api(state, &connection.scope_value);
            handle_issue_event(state, &api, &connection, &envelope).await
        }
        other => {
            debug!(event_type = other, "Unsupported Linear event type");
            Ok(())
        }
    }
}

async fn handle_issue_event(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &crate::adapters::models::AdapterConnection,
    envelope: &LinearWebhookEnvelope,
) -> Result<(), AdapterError> {
    // Only act on state updates.
    if envelope.action != "update" {
        return Ok(());
    }

    let Some(data) = envelope.data.as_ref() else {
        debug!("Issue.update missing data — ignoring");
        return Ok(());
    };

    let issue: super::webhook_payloads::IssuePayload =
        match serde_json::from_value(data.clone()) {
            Ok(i) => i,
            Err(e) => {
                debug!(error = %e, "Failed to parse IssuePayload — ignoring");
                return Ok(());
            }
        };

    let Some(team_id) = issue.resolved_team_id().map(str::to_string) else {
        debug!(issue = %issue.identifier, "Issue.update missing team_id — ignoring");
        return Ok(());
    };

    let rules = agent_runtime::resolve_rules_for_team(state, connection, &team_id).await?;
    if rules.is_empty() {
        debug!(team_id = %team_id, "No active Linear rule for team — ignoring Issue.update");
        return Ok(());
    }

    agent_runtime::handle_issue_update(
        state,
        api,
        connection,
        &rules,
        envelope.updated_from.as_ref(),
        &issue,
    )
    .await
}

async fn handle_agent_session_event(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &crate::adapters::models::AdapterConnection,
    envelope: &LinearWebhookEnvelope,
) -> Result<(), AdapterError> {
    let Some(session) = envelope.agent_session.as_ref() else {
        debug!("AgentSessionEvent missing agentSession payload — ignoring");
        return Ok(());
    };

    let Some(issue) = session.issue.as_ref() else {
        debug!(session_id = %session.id, "AgentSessionEvent missing issue — ignoring");
        return Ok(());
    };

    let team_id = issue
        .team_id
        .clone()
        .or_else(|| issue.team.as_ref().map(|t| t.id.clone()))
        .unwrap_or_default();
    let rules = agent_runtime::resolve_rules_for_team(state, connection, &team_id).await?;

    match envelope.action.as_str() {
        "created" => {
            if let Err(e) =
                agent_runtime::handle_session_created(state, api, connection, &rules, session).await
            {
                warn!(session_id = %session.id, error = %e, "handle_session_created failed");
            }
        }
        "prompted" => {
            let body = envelope
                .agent_activity
                .as_ref()
                .map_or("", |a| a.body.as_str());
            if body.is_empty() {
                debug!(session_id = %session.id, "Empty prompted body — ignoring");
                return Ok(());
            }
            if let Err(e) =
                agent_runtime::handle_session_prompted(state, api, connection, &session.id, body)
                    .await
            {
                warn!(session_id = %session.id, error = %e, "handle_session_prompted failed");
            }
        }
        other => {
            debug!(action = other, "Unsupported AgentSessionEvent action");
        }
    }

    Ok(())
}

fn build_agent_api(state: &AppState, org_id: &str) -> LinearAgentApi {
    LinearAgentApi::new(
        default_http_client(),
        state.db.clone(),
        org_id.to_string(),
        state.config.linear.oauth_client_id.clone(),
        state.config.linear.oauth_client_secret.clone(),
    )
}
