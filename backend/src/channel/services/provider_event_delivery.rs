use crate::channel::error::ChannelError;
use crate::channel::inbound::CommandInfo;
use crate::channel::models::{InboundMessage, ProviderConnection, TaskTriggerRoute};
use crate::state::AppState;

pub struct ProviderEventDeliveryService<'a> {
    state: &'a AppState,
}

impl<'a> ProviderEventDeliveryService<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn deliver(
        &self,
        connection: &ProviderConnection,
        trigger: &TaskTriggerRoute,
        message: InboundMessage,
        command: Option<CommandInfo>,
        provider_label: &str,
        subject_kind: &str,
        subject_key: &str,
        destination_config: serde_json::Value,
    ) -> Result<String, ChannelError> {
        crate::channel::binding_runtime::route_inbound_message(
            self.state,
            connection,
            trigger,
            message,
            command,
            provider_label,
            subject_kind,
            subject_key,
            destination_config,
        )
        .await
    }
}
