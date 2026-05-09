use crate::channel::error::ChannelError;
use crate::channel::provider::ChannelProviders;
use crate::state::AppState;

pub struct DirectSendService<'a> {
    state: &'a AppState,
    providers: &'a ChannelProviders,
}

impl<'a> DirectSendService<'a> {
    pub fn new(state: &'a AppState, providers: &'a ChannelProviders) -> Self {
        Self { state, providers }
    }

    pub async fn send_to_trigger(
        &self,
        trigger_id: &str,
        task_id: Option<&str>,
        text: &str,
        file_ids: &[String],
    ) -> Result<Option<String>, ChannelError> {
        crate::channel::outbound::direct_send_to_rule(
            self.state,
            self.providers,
            trigger_id,
            task_id,
            text,
            file_ids,
        )
        .await
    }
}
