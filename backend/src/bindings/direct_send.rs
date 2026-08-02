use crate::adapters::error::AdapterError;
use crate::adapters::Adapters;
use crate::state::AppState;

pub struct DirectSendService<'a> {
    state: &'a AppState,
    providers: &'a Adapters,
}

impl<'a> DirectSendService<'a> {
    pub fn new(state: &'a AppState, providers: &'a Adapters) -> Self {
        Self { state, providers }
    }

    pub async fn send_to_trigger(
        &self,
        loop_id: &str,
        task_id: Option<&str>,
        text: &str,
        file_ids: &[String],
    ) -> Result<Option<String>, AdapterError> {
        crate::bindings::outbound::direct_send_to_rule(
            self.state,
            self.providers,
            loop_id,
            task_id,
            text,
            file_ids,
        )
        .await
    }
}
