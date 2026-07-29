use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use tracing::{debug, warn};

use crate::adapters::error::AdapterError;
use crate::adapters::inbound_delivery::{admit_inbound_message, InboundDeliveryPayload};
use crate::adapters::metrics::record_webhook;
use crate::adapters::models::InboundMessage;
use crate::adapters::Adapter;
use crate::state::AppState;

/// The idempotency key for one Slack delivery.
///
/// Slack repeats the envelope `event_id` on every retry of the same event,
/// which is exactly what the durable dedup index on
/// `(adapter, kind, event_id)` needs. The fallback covers the envelope that
/// carries no `event_id` at all — the channel plus the message's own external
/// id names the same message just as stably, and is strictly better than the
/// old in-memory cache, which skipped deduplication entirely for those.
fn admission_event_id(payload: &serde_json::Value, msg: &InboundMessage) -> String {
    payload
        .get("event_id")
        .and_then(|v| v.as_str())
        .filter(|id| !id.is_empty())
        .map_or_else(
            || {
                let message_id = msg
                    .external_event_id
                    .as_deref()
                    .unwrap_or(&msg.message_thread_id);
                format!("{}:{}", msg.source_id, message_id)
            },
            ToString::to_string,
        )
}

#[utoipa::path(
    post,
    path = "/api/v1/adapters/slack/webhook",
    tag = "adapters",
    request_body(content = String),
    responses((status = 200, description = "Webhook processed"))
)]
pub async fn slack_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::response::Response, AdapterError> {
    use axum::response::IntoResponse;

    let providers = state
        .adapters
        .as_ref()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("slack".into()))?;

    let slack_provider = providers
        .slack()
        .ok_or_else(|| AdapterError::AdapterNotConfigured("slack".into()))?;

    let payload: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|e| AdapterError::BadRequest(format!("Invalid JSON: {e}")))?;

    let event_type = payload
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    debug!(event_type, "Slack webhook received");

    if event_type == "url_verification" {
        let timestamp = headers
            .get("x-slack-request-timestamp")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let sig = headers
            .get("x-slack-signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        super::signature::verify_signature(slack_provider.signing_secret(), timestamp, &body, sig)?;

        let challenge = payload
            .get("challenge")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        return Ok((StatusCode::OK, challenge.to_string()).into_response());
    }

    let source_id = payload
        .pointer("/event/channel")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    if source_id.is_empty() {
        let event_subtype = payload
            .pointer("/event/type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        warn!(
            event_type,
            event_subtype, "Slack webhook has no channel ID in event payload, ignoring"
        );
        record_webhook("slack", "ignored", "no_channel", event_type, "", "");
        return Ok(StatusCode::OK.into_response());
    }

    let team_id = payload
        .get("team_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("default");
    let Some((connection, rule)) =
        crate::bindings::runtime::resolve_chat_provider_event(&state, "slack", team_id, source_id)
            .await?
    else {
        debug!(
            source_id,
            team_id, "Slack message ignored: no task trigger for this source"
        );
        record_webhook("slack", "ignored", "no_trigger", event_type, "", "");
        return Ok(StatusCode::OK.into_response());
    };
    let config = crate::bindings::runtime::trigger_context(&rule, "slack");

    let thread_ts = payload.pointer("/event/thread_ts").and_then(|v| v.as_str());
    let is_tracked_thread = if let Some(tts) = thread_ts {
        let destination = slack_provider.binding_destination_from_existing_thread(
            source_id,
            tts,
            &serde_json::json!({
                "channel": source_id,
                "thread_ts": tts,
                "team_id": team_id,
            }),
            None,
        )?;
        state
            .task_bindings
            .find_by_subject(
                slack_provider.provider_key(),
                destination.subject_kind,
                &destination.subject_key,
            )
            .await
            .is_ok_and(|rows| !rows.is_empty())
    } else {
        false
    };

    let result = slack_provider
        .parse_event(&headers, &body, &config, &state.db, is_tracked_thread)
        .await?;

    match result {
        super::adapter::SlackWebhookResult::Challenge(challenge) => {
            Ok((StatusCode::OK, challenge).into_response())
        }
        super::adapter::SlackWebhookResult::Message(msg) => {
            let msg = *msg;
            let destination = match slack_provider.binding_destination_from_existing_thread(
                &msg.source_id,
                &msg.message_thread_id,
                &msg.thread_properties,
                None,
            ) {
                Ok(destination) => destination,
                Err(e) => {
                    warn!(error = %e, "Failed to build Slack binding destination");
                    record_webhook(
                        "slack",
                        "error",
                        "internal",
                        event_type,
                        &rule.id,
                        &connection.id,
                    );
                    return Ok(StatusCode::OK.into_response());
                }
            };
            let event_id = admission_event_id(&payload, &msg);
            let work = InboundDeliveryPayload {
                connection_id: connection.id.clone(),
                rule_id: rule.id.clone(),
                subject_kind: destination.subject_kind.to_string(),
                subject_key: destination.subject_key,
                destination_config: destination.destination_config,
                event_type: event_type.to_string(),
                message: msg,
            };

            // Everything below the 200 depends on this write: a failure here
            // must reach Slack as a failure so it re-delivers, and must never
            // be swallowed into an acknowledgment. The routing itself now runs
            // in the Adapter delivery worker, which is also where the webhook
            // outcome metric is recorded.
            let admission = admit_inbound_message(
                &state,
                super::delivery::ADAPTER,
                super::delivery::INBOUND_MESSAGE,
                event_id.clone(),
                work,
            )
            .await?;
            debug!(
                source_id,
                event_id = %event_id,
                admission = admission.as_str(),
                "Slack inbound message admitted for routing"
            );
            Ok(StatusCode::OK.into_response())
        }
        super::adapter::SlackWebhookResult::Ignore(reason) => {
            debug!(source_id, reason, "Slack event ignored by provider");
            record_webhook(
                "slack",
                "ignored",
                reason,
                event_type,
                &rule.id,
                &connection.id,
            );
            Ok(StatusCode::OK.into_response())
        }
    }
}
