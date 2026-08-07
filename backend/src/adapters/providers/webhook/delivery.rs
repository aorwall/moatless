//! The generic Webhook Adapter's registration on the durable Adapter delivery
//! seam.

use std::sync::Arc;

use crate::adapters::delivery::AdapterDeliveryHandlers;
use crate::adapters::inbound_delivery::InboundMessageHandler;
use crate::state::AppState;

/// Adapter name on `adapter_delivery` rows, metrics, and bindings.
pub(crate) const ADAPTER: &str = "webhook";

/// The subject a webhook binding is keyed on: one thread on one trigger.
pub(crate) const SUBJECT_KIND: &str = "webhook_thread";

/// Routing one inbound webhook message.
///
/// Namespaced under `webhook.` so kinds stay owned by exactly one adapter and
/// two migrations never contend for the same name.
pub(crate) const INBOUND_MESSAGE: &str = "webhook.inbound_message";

/// Register the webhook deliveries this build knows how to execute.
///
/// The worker claims only registered kinds, so a replica that predates this
/// registration leaves webhook deliveries for one that has it rather than failing
/// them.
pub fn register(handlers: &mut AdapterDeliveryHandlers, state: &AppState) {
    handlers.register(
        INBOUND_MESSAGE,
        Arc::new(InboundMessageHandler::new(ADAPTER, state.routing.clone())),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kind is half of the durable dedup key and is matched by string at
    /// both ends, so renaming it orphans every record already admitted.
    #[test]
    fn the_kind_is_namespaced_to_this_adapter() {
        assert_eq!(INBOUND_MESSAGE, "webhook.inbound_message");
        assert!(INBOUND_MESSAGE.starts_with(&format!("{ADAPTER}.")));
    }
}
