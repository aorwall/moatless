use moatless_api_schemas::channel::SendTaskReplyResponse;

use crate::channel::dao::bindings::PgTaskBindingRepository;
use crate::channel::error::ChannelError;
use crate::channel::handlers::common::{binding_descriptor, load_provider_connection_for_auth};
use crate::channel::outbound;
use crate::scope::ScopeContext;
use crate::state::AppState;

pub struct ReplyService<'a> {
    state: &'a AppState,
}

impl<'a> ReplyService<'a> {
    pub fn new(state: &'a AppState) -> Self {
        Self { state }
    }

    pub async fn send_reply(
        &self,
        ctx: &ScopeContext,
        task_id: &str,
        binding_id: Option<&str>,
        text: &str,
        file_ids: &[String],
    ) -> Result<SendTaskReplyResponse, ChannelError> {
        let providers = self.state.channel_providers.as_ref().ok_or_else(|| {
            ChannelError::ProviderNotConfigured("No channel providers configured".into())
        })?;

        let repo = PgTaskBindingRepository(self.state.db.clone());
        let bindings = if let Some(binding_id) = binding_id {
            let binding = repo
                .find_by_id(binding_id)
                .await?
                .ok_or_else(|| ChannelError::NotFound(format!("Binding {binding_id} not found")))?;
            if binding.task_id != task_id {
                return Err(ChannelError::BadRequest(format!(
                    "Binding {binding_id} does not belong to task {task_id}"
                )));
            }
            load_provider_connection_for_auth(self.state, ctx, &binding.connection_id).await?;
            vec![binding]
        } else {
            repo.find_by_task_id(task_id).await?
        };

        match bindings.len() {
            0 => Err(ChannelError::ConfigNotFound(format!(
                "Task {task_id} has no bindings"
            ))),
            1 => {
                let binding = &bindings[0];
                let (sent_count, errors) = outbound::send_to_binding(
                    self.state,
                    providers,
                    task_id,
                    &binding.id,
                    text,
                    file_ids,
                )
                .await?;
                if sent_count == 0 && !errors.is_empty() {
                    return Err(ChannelError::ProviderApi(errors.join("; ")));
                }
                Ok(SendTaskReplyResponse {
                    sent_count,
                    errors,
                    targets: if sent_count > 0 {
                        vec![binding_descriptor(binding)]
                    } else {
                        vec![]
                    },
                })
            }
            _ => Err(ChannelError::MultipleTargets {
                targets: bindings.iter().map(binding_descriptor).collect(),
            }),
        }
    }
}
