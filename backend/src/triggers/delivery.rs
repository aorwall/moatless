use crate::adapters::error::AdapterError;
use crate::adapters::models::{AdapterConnection, InboundMessage, TaskTriggerRoute};
use crate::state::AppState;

pub struct AdapterEventDeliveryService<'a> {
    state: &'a AppState,
}

impl<'a> AdapterEventDeliveryService<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn deliver(
        &self,
        connection: &AdapterConnection,
        trigger: &TaskTriggerRoute,
        message: InboundMessage,
        provider_label: &str,
        subject_kind: &str,
        subject_key: &str,
        destination_config: serde_json::Value,
    ) -> Result<String, AdapterError> {
        crate::bindings::runtime::route_inbound_message(
            self.state,
            connection,
            trigger,
            message,
            provider_label,
            subject_kind,
            subject_key,
            destination_config,
        )
        .await
    }
}
