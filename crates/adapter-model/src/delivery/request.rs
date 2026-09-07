//! Durable inbound request encoding.

use super::AdapterDeliveryRequest;
use crate::error::{AdapterError, SharedAdapterError};
use moatless_types::adapter::InboundDeliveryPayload;

/// Build the durable request for one inbound message, keyed by the sender's
/// own delivery id.
///
/// Admitting it is a separate call every caller makes for itself. That split is
/// deliberate: admitting here would flatten the `AdmitError` into an
/// `AdapterError` on the way out (`.admit(..).await?`) and make the shared
/// "not admitted" ingress variant unreachable for every Adapter that went
/// through it. What is left here is the part worth sharing:
/// the payload shape and the one serialization error, spelled once instead of
/// at each of the three call sites.
pub fn inbound_delivery_request(
    adapter: &'static str,
    kind: &'static str,
    event_id: String,
    payload: InboundDeliveryPayload,
) -> Result<AdapterDeliveryRequest, AdapterError> {
    let payload = serde_json::to_value(payload).map_err(|e| {
        SharedAdapterError::Internal(format!("Failed to serialize inbound delivery payload: {e}"))
    })?;
    Ok(AdapterDeliveryRequest {
        adapter,
        kind,
        event_id,
        payload,
    })
}
