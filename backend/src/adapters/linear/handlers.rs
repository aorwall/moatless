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

    let linear_provider = providers
        .linear()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("linear".into()))?;

    match dispatch_webhook(&state, linear_provider, &headers, &body).await {
        Ok(event_type) => {
            record_webhook("linear", "success", "", &event_type, "", "");
        }
        Err(e) => {
            warn!(error = %e, "Linear webhook processing error");
            record_webhook("linear", "error", "delivery_failed", "", "", "");
        }
    }

    Ok(StatusCode::OK)
}

/// Verify the webhook, parse the envelope, and route based on `type`.
///
/// Returns the parsed `event_type` (empty string when the envelope failed to
/// parse) so the caller can label the success-path metric. Always returns
/// `Ok(_)` on benign rejections (unknown org, dedup hit, unsupported event).
async fn dispatch_webhook(
    state: &AppState,
    provider: &LinearAdapter,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<String, AdapterError> {
    let signature = headers
        .get("linear-signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let signature_present = !signature.is_empty();

    let envelope: LinearWebhookEnvelope = serde_json::from_slice(body).map_err(|e| {
        info!(
            error = %e,
            body_len = body.len(),
            signature_present,
            "Linear webhook: failed to parse envelope"
        );
        AdapterError::BadRequest(format!("Invalid Linear webhook payload: {e}"))
    })?;

    info!(
        event_type = %envelope.event_type,
        action = %envelope.action,
        org_id = envelope.organization_id.as_deref().unwrap_or(""),
        webhook_id = envelope.webhook_id.as_deref().unwrap_or(""),
        body_len = body.len(),
        signature_present,
        "Linear webhook received"
    );

    if !signature_present {
        info!("Linear webhook ignored: missing Linear-Signature header");
        return Ok(envelope.event_type.clone());
    }

    let Some(org_id) = envelope.organization_id.as_deref() else {
        info!("Linear webhook ignored: missing organizationId in envelope");
        return Ok(envelope.event_type.clone());
    };

    let connection = PgAdapterConnectionRepository(state.db.clone())
        .find_by_external_account("linear", org_id)
        .await?;
    let Some(connection) = connection else {
        info!(
            org_id,
            "Linear webhook ignored: no integration_connection registered"
        );
        return Ok(envelope.event_type.clone());
    };

    let secret = connection
        .webhook_secret
        .as_deref()
        .or(provider.global_webhook_secret_for_dispatch());
    let Some(secret) = secret else {
        warn!(
            org_id,
            "Linear webhook rejected: no webhook secret configured on connection or env"
        );
        return Err(AdapterError::SignatureInvalid);
    };

    if let Err(e) = super::signature::verify_signature(secret, body, signature) {
        warn!(
            org_id,
            event_type = %envelope.event_type,
            webhook_id = envelope.webhook_id.as_deref().unwrap_or(""),
            secret_source = if connection.webhook_secret.is_some() { "connection" } else { "env" },
            signature_len = signature.len(),
            "Linear webhook rejected: signature verification failed"
        );
        return Err(e);
    }

    // Dedup on the specific resource/event identity, NOT Linear's `webhookId`
    // (which is the stable webhook-config id, identical on every delivery — see
    // `dedup_key`). Compute unconditionally: a resource id can exist even when
    // `webhook_id` is absent.
    let key = dedup_key(&envelope);
    if !provider.mark_seen_for_dispatch(&key).await {
        info!(
            webhook_id = envelope.webhook_id.as_deref().unwrap_or(""),
            event_type = %envelope.event_type,
            org_id,
            "Linear webhook deduplicated"
        );
        return Ok(envelope.event_type.clone());
    }

    let event_type = envelope.event_type.clone();
    let result = match event_type.as_str() {
        "AgentSessionEvent" => {
            let api = build_agent_api(state, &connection.scope_value);
            handle_agent_session_event(state, &api, &connection, &envelope).await
        }
        "Issue" => {
            let api = build_agent_api(state, &connection.scope_value);
            handle_issue_event(state, &api, &connection, &envelope).await
        }
        "Comment" => {
            let api = build_agent_api(state, &connection.scope_value);
            handle_comment_event(state, &api, &connection, &envelope).await
        }
        "AppUserNotification" => {
            let api = build_agent_api(state, &connection.scope_value);
            handle_app_user_notification(state, &api, &connection, &envelope).await
        }
        other => {
            let data_keys: Vec<&str> = envelope
                .data
                .as_ref()
                .and_then(|v| v.as_object())
                .map(|m| m.keys().map(String::as_str).collect())
                .unwrap_or_default();
            warn!(
                event_type = other,
                action = %envelope.action,
                org_id,
                webhook_id = envelope.webhook_id.as_deref().unwrap_or(""),
                actor_id = envelope.actor.as_ref().map_or("", |a| a.id.as_str()),
                actor_name = envelope.actor.as_ref().map_or("", |a| a.name.as_str()),
                data_keys = ?data_keys,
                body_len = body.len(),
                "Linear webhook: unsupported event type"
            );
            Ok(())
        }
    };
    result.map(|()| event_type)
}

/// Derive a stable dedup identity for a Linear webhook delivery.
///
/// Linear's `webhookId` is the *webhook-configuration* id — it is identical on
/// every delivery, not unique per event — so keying dedup on it (alone or with
/// `event_type`) silently drops distinct events delivered within the cache TTL.
/// Instead we key on the most specific stable *resource* identity available,
/// plus the action (so e.g. `created` vs `prompted` on one session don't
/// collide).
///
/// Resource id by event type:
/// - `AgentSessionEvent` → `agentSession.id`
/// - `Comment` / `Issue`  → `data.id` (resource UUID)
/// - `AppUserNotification` → `notification.id`
/// - fallback (unknown / missing) → `webhook_id:webhook_timestamp`
///   (Linear resends the same `webhookTimestamp` on genuine retries).
///
/// Key format: `"{event_type}:{action}:{resource_id}"`.
fn dedup_key(envelope: &LinearWebhookEnvelope) -> String {
    let resource_id = match envelope.event_type.as_str() {
        "AgentSessionEvent" => envelope.agent_session.as_ref().map(|s| s.id.clone()),
        "Comment" | "Issue" => envelope
            .data
            .as_ref()
            .and_then(|d| d.get("id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        "AppUserNotification" => envelope
            .notification
            .as_ref()
            .and_then(|n| n.get("id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        _ => None,
    };

    let resource_id = resource_id.unwrap_or_else(|| {
        format!(
            "{}:{}",
            envelope.webhook_id.as_deref().unwrap_or(""),
            envelope.webhook_timestamp.unwrap_or(0)
        )
    });

    format!(
        "{}:{}:{}",
        envelope.event_type, envelope.action, resource_id
    )
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

    let issue: super::webhook_payloads::IssuePayload = match serde_json::from_value(data.clone()) {
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

async fn handle_comment_event(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &crate::adapters::models::AdapterConnection,
    envelope: &LinearWebhookEnvelope,
) -> Result<(), AdapterError> {
    if envelope.action != "create" {
        debug!(action = %envelope.action, "Comment event with non-create action — ignoring");
        return Ok(());
    }

    let Some(data) = envelope.data.as_ref() else {
        debug!("Comment.create missing data — ignoring");
        return Ok(());
    };

    let comment: super::webhook_payloads::CommentPayloadData =
        match serde_json::from_value(data.clone()) {
            Ok(c) => c,
            Err(e) => {
                debug!(error = %e, "Failed to parse CommentPayloadData — ignoring");
                return Ok(());
            }
        };

    let Some(issue_id) = comment.resolved_issue_id().map(str::to_string) else {
        debug!(comment_id = %comment.id, "Comment.create missing issueId — ignoring");
        return Ok(());
    };

    agent_runtime::handle_comment_created(state, api, connection, &issue_id, &comment).await
}

async fn handle_app_user_notification(
    state: &AppState,
    api: &LinearAgentApi,
    connection: &crate::adapters::models::AdapterConnection,
    envelope: &LinearWebhookEnvelope,
) -> Result<(), AdapterError> {
    let Some(raw) = envelope.notification.as_ref() else {
        debug!("AppUserNotification missing notification payload — ignoring");
        return Ok(());
    };

    let notif: super::webhook_payloads::AppUserNotificationData =
        match serde_json::from_value(raw.clone()) {
            Ok(n) => n,
            Err(e) => {
                debug!(error = %e, "Failed to parse AppUserNotificationData — ignoring");
                return Ok(());
            }
        };

    match envelope.action.as_str() {
        "issueStatusChanged" => {
            agent_runtime::handle_issue_status_notification(state, api, connection, &notif).await
        }
        other => {
            warn!(
                action = other,
                org_id = connection.scope_value.as_str(),
                notification_id = notif.id.as_deref().unwrap_or(""),
                notification_kind = notif.kind.as_str(),
                "Linear AppUserNotification: action not handled, ignoring"
            );
            Ok(())
        }
    }
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
            warn!(
                action = other,
                session_id = %session.id,
                "Unsupported AgentSessionEvent action"
            );
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

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(raw: serde_json::Value) -> LinearWebhookEnvelope {
        serde_json::from_value(raw).expect("parse envelope")
    }

    #[test]
    fn distinct_agent_sessions_sharing_a_webhook_id_get_distinct_keys() {
        // The bug: Linear reuses one stable `webhookId` across every delivery.
        // Two distinct sessions must NOT collide — previously the second was
        // dropped as a "duplicate" within the 5-minute TTL.
        let a = envelope(serde_json::json!({
            "action": "created",
            "type": "AgentSessionEvent",
            "webhookId": "9628c862-stable",
            "agentSession": {"id": "session-A"}
        }));
        let b = envelope(serde_json::json!({
            "action": "created",
            "type": "AgentSessionEvent",
            "webhookId": "9628c862-stable",
            "agentSession": {"id": "session-B"}
        }));
        assert_ne!(dedup_key(&a), dedup_key(&b));
    }

    #[test]
    fn same_session_and_action_collapses_retries() {
        let first = envelope(serde_json::json!({
            "action": "created",
            "type": "AgentSessionEvent",
            "webhookId": "9628c862-stable",
            "agentSession": {"id": "session-A"}
        }));
        let retry = envelope(serde_json::json!({
            "action": "created",
            "type": "AgentSessionEvent",
            "webhookId": "9628c862-stable",
            "agentSession": {"id": "session-A"}
        }));
        assert_eq!(dedup_key(&first), dedup_key(&retry));
    }

    #[test]
    fn same_session_created_vs_prompted_get_distinct_keys() {
        let created = envelope(serde_json::json!({
            "action": "created",
            "type": "AgentSessionEvent",
            "agentSession": {"id": "session-A"}
        }));
        let prompted = envelope(serde_json::json!({
            "action": "prompted",
            "type": "AgentSessionEvent",
            "agentSession": {"id": "session-A"}
        }));
        assert_ne!(dedup_key(&created), dedup_key(&prompted));
    }

    #[test]
    fn comment_and_agent_session_sharing_a_webhook_id_get_distinct_keys() {
        // Comment.create and AgentSessionEvent.created in one fan-out share a
        // `webhookId` but have different resource ids *and* event types.
        let comment = envelope(serde_json::json!({
            "action": "create",
            "type": "Comment",
            "webhookId": "shared-id",
            "data": {"id": "comment-1", "body": "hi"}
        }));
        let session = envelope(serde_json::json!({
            "action": "created",
            "type": "AgentSessionEvent",
            "webhookId": "shared-id",
            "agentSession": {"id": "session-1"}
        }));
        assert_ne!(dedup_key(&comment), dedup_key(&session));
    }

    #[test]
    fn unknown_event_without_resource_id_falls_back_to_webhook_id_and_timestamp() {
        let a = envelope(serde_json::json!({
            "action": "something",
            "type": "UnknownThing",
            "webhookId": "wh-1",
            "webhookTimestamp": 1_700_000_000_000i64
        }));
        let b = envelope(serde_json::json!({
            "action": "something",
            "type": "UnknownThing",
            "webhookId": "wh-1",
            "webhookTimestamp": 1_700_000_000_001i64
        }));
        // Distinct deliveries (different timestamps) → distinct keys.
        assert_ne!(dedup_key(&a), dedup_key(&b));
        // The fallback key embeds both the webhook id and the timestamp.
        let key = dedup_key(&a);
        assert!(key.contains("wh-1"), "key should contain webhook_id: {key}");
        assert!(
            key.contains("1700000000000"),
            "key should contain webhook_timestamp: {key}"
        );
        // A genuine retry resends the same timestamp → same key (collapses).
        let retry = envelope(serde_json::json!({
            "action": "something",
            "type": "UnknownThing",
            "webhookId": "wh-1",
            "webhookTimestamp": 1_700_000_000_000i64
        }));
        assert_eq!(dedup_key(&a), dedup_key(&retry));
    }
}
