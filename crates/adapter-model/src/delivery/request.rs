//! Durable inbound request encoding.

use super::AdapterDeliveryRequest;
use crate::error::{AdapterError, SharedAdapterError};
use moatless_types::adapter::InboundDeliveryPayload;

/// Build the durable request for one inbound message, keyed by the sender's own
/// delivery id.
///
/// Admission is a separate call each caller makes itself: doing it here would
/// flatten `AdmitError` and make the "not admitted" ingress variant
/// unreachable.
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
