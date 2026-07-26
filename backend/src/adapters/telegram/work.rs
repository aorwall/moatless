//! Telegram's registration on the durable adapter work seam.

use std::sync::Arc;

use crate::adapters::inbound_work::InboundMessageHandler;
use crate::adapters::work::AdapterWorkHandlers;
use crate::state::AppState;

/// Provider label on `adapter_work` rows, metrics, and bindings.
pub(crate) const PROVIDER: &str = "telegram";

/// Routing one inbound Telegram message.
///
/// Namespaced under `telegram.` so kinds stay owned by exactly one provider and
/// two migrations never contend for the same name.
pub(crate) const INBOUND_MESSAGE: &str = "telegram.inbound_message";

/// Register the Telegram work this build knows how to execute.
///
/// The worker claims only registered kinds, so a replica that predates this
/// registration leaves Telegram work for one that has it rather than failing
/// it.
pub fn register(handlers: &mut AdapterWorkHandlers, state: &AppState) {
    handlers.register(
        INBOUND_MESSAGE,
        Arc::new(InboundMessageHandler::new(PROVIDER, state.clone())),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The kind is half of the durable dedup key and is matched by string at
    /// both ends, so renaming it orphans every record already admitted.
    #[test]
    fn the_kind_is_namespaced_to_this_provider() {
        assert_eq!(INBOUND_MESSAGE, "telegram.inbound_message");
        assert!(INBOUND_MESSAGE.starts_with(&format!("{PROVIDER}.")));
    }
}
