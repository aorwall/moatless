//! The Moatless side of the compatibility module's write ports.
//!
//! One implementation each of [`AgentCatalogSource`] and [`ThreadCommandSink`],
//! over [`AppState`]. Everything here is translation: a T3 command has already
//! been validated by `super::commands` by the time it arrives, and each method
//! turns it into the same service call the Moatless UI makes.

use std::sync::Arc;

use async_trait::async_trait;
use moatless_api_schemas::agent::AgentDescriptor;
use moatless_backend_core::scope::ScopeContext;

use super::ports::{
    AgentCatalogSource, CommandRefusal, NewThread, QuestionAnswer, ThreadCommandSink, ThreadMeta,
    ThreadTurn,
};
use crate::message::error::MessageError;
use crate::message::service as message_service;
use crate::state::AppState;
use crate::task::dao::TaskPatch;
use crate::task::error::TaskError;
use crate::task::models::TaskStatus;
use crate::task::service as task_service;

pub struct AppStateAgents(pub AppState);

#[async_trait]
impl AgentCatalogSource for AppStateAgents {
    async fn list(&self, viewer: &ScopeContext) -> Vec<AgentDescriptor> {
        crate::agent::service::list_agents(&self.0, viewer).await
    }
}

pub struct AppStateCommands(pub AppState);

impl AppStateCommands {
    pub fn new(state: AppState) -> Arc<Self> {
        Arc::new(Self(state))
    }

    /// A viewer as the message and task services expect one.
    ///
    /// `is_agent_context` is false because every command on this socket comes
    /// from a person looking at a screen — which is what makes the auto-follow
    /// on send correct here.
    fn sender(viewer: &ScopeContext) -> message_service::Sender<'_> {
        message_service::Sender {
            user_id: &viewer.user_id,
            scope: viewer,
            is_agent_context: false,
        }
    }

    async fn ensure_followed_open(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<(), CommandRefusal> {
        let Some(task) = self.0.tasks.find_by_id(task_id, viewer).await.map_err(|error| {
            tracing::warn!(%error, task_id, "T3 settlement command could not resolve its thread");
            CommandRefusal::Failed {
                message: "Could not reach that thread.".to_string(),
            }
        })?
        else {
            return Err(CommandRefusal::NotWritable {
                task_id: task_id.to_string(),
            });
        };
        if task.status != TaskStatus::Open {
            return Err(CommandRefusal::NotWritable {
                task_id: task_id.to_string(),
            });
        }
        let followed = self
            .0
            .tasks
            .get_followed(&viewer.user_id, &[task_id.to_string()])
            .await
            .map_err(|error| {
                tracing::warn!(%error, task_id, "T3 settlement command could not resolve follow");
                CommandRefusal::Failed {
                    message: "Could not reach that thread.".to_string(),
                }
            })?;
        if !followed.contains(task_id) {
            return Err(CommandRefusal::NotWritable {
                task_id: task_id.to_string(),
            });
        }
        Ok(())
    }

    fn state_refusal(task_id: &str, error: sqlx::Error) -> CommandRefusal {
        tracing::warn!(%error, task_id, "T3 settlement state update failed");
        CommandRefusal::Failed {
            message: "Could not update that thread.".to_string(),
        }
    }
}

#[async_trait]
impl ThreadCommandSink for AppStateCommands {
    async fn send_turn(
        &self,
        viewer: &ScopeContext,
        turn: ThreadTurn,
    ) -> Result<(), CommandRefusal> {
        // A replayed frame is resolved inside the send path — it reads as sent
        // and writes nothing — so there is no already-persisted case to handle
        // here.
        message_service::send_user_message(
            &self.0,
            &Self::sender(viewer),
            &turn.task_id,
            message_service::UserMessage {
                text: turn.text,
                model: turn.model,
                agent_mode: turn.agent_mode,
                resume: true,
                message_uuid: Some(turn.message_uuid),
                ..Default::default()
            },
        )
        .await
        .map(|_| ())
        .map_err(|e| send_refusal(&turn.task_id, e))
    }

    async fn create_thread(
        &self,
        viewer: &ScopeContext,
        thread: NewThread,
    ) -> Result<(), CommandRefusal> {
        // The transport is not selectable from T3, so a thread created there
        // runs the first catalog agent of the kind the person picked.
        let agent_type = default_agent_for(&thread.instance);

        let request = moatless_api_schemas::task::CreateTaskRequest {
            repository_id: None,
            workspace_id: Some(thread.workspace_id),
            agent_type: Some(agent_type),
            name: thread.title,
            description: None,
            branch: None,
            message: thread.first_message.as_ref().map(|m| m.text.clone()),
            model: thread.model,
            agent_mode: thread
                .first_message
                .as_ref()
                .and_then(|m| m.agent_mode.clone()),
            contexts: vec![],
            file_ids: vec![],
            parent_task_id: None,
            headless: Some(false),
        };

        let mut intent = task_service::CreateTaskIntent::new(request);
        intent.task_id = Some(thread.task_id.clone());
        // A thread created with nothing in it gets no sandbox until something
        // is sent to it.
        intent.provision = thread.first_message.is_some();
        // T3's optimistic copy of the first message is keyed on the id it
        // minted, so that id has to be the one persisted.
        intent.first_message_uuid = thread.first_message.map(|m| m.message_uuid);

        task_service::create_task(&self.0, viewer, &viewer.user_id, intent, |_| async {
            serde_json::Value::Null
        })
        .await
        .map(|_| ())
        .map_err(|e| create_refusal(&thread.task_id, e))
    }

    async fn update_meta(
        &self,
        viewer: &ScopeContext,
        meta: ThreadMeta,
    ) -> Result<(), CommandRefusal> {
        if meta.title.is_none() && meta.model.is_none() {
            return Ok(());
        }
        let patch = TaskPatch {
            name: meta.title,
            model: meta.model,
            ..Default::default()
        };
        let updated = self
            .0
            .tasks
            .update_task(&meta.task_id, &patch, viewer)
            .await
            .map_err(|error| {
                tracing::warn!(%error, task_id = %meta.task_id, "T3 meta update failed");
                CommandRefusal::Failed {
                    message: "Could not update that thread.".to_string(),
                }
            })?;
        // `update_task` carries the access rule in its `WHERE`, so a viewer who
        // may not control the Task gets a clean `false` rather than an error.
        // Reporting that as success would leave T3 showing a title nothing
        // stored.
        if !updated {
            return Err(CommandRefusal::NotWritable {
                task_id: meta.task_id,
            });
        }
        Ok(())
    }

    async fn stop(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        task_service::stop_task(&self.0, viewer, task_id)
            .await
            .map_err(|e| create_refusal(task_id, e))
    }

    async fn settle(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.ensure_followed_open(viewer, task_id).await?;
        self.0
            .tasks
            .settle_task(&viewer.user_id, task_id, chrono::Utc::now())
            .await
            .map_err(|e| Self::state_refusal(task_id, e))
    }

    async fn unsettle(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.ensure_followed_open(viewer, task_id).await?;
        self.0
            .tasks
            .unsettle_task(&viewer.user_id, task_id)
            .await
            .map_err(|e| Self::state_refusal(task_id, e))
    }

    async fn snooze(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        snoozed_until: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), CommandRefusal> {
        self.ensure_followed_open(viewer, task_id).await?;
        self.0
            .tasks
            .snooze_task(&viewer.user_id, task_id, snoozed_until, chrono::Utc::now())
            .await
            .map_err(|e| Self::state_refusal(task_id, e))
    }

    async fn unsnooze(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.ensure_followed_open(viewer, task_id).await?;
        self.0
            .tasks
            .unsnooze_task(&viewer.user_id, task_id)
            .await
            .map_err(|e| Self::state_refusal(task_id, e))
    }

    async fn answer_question(
        &self,
        viewer: &ScopeContext,
        answer: QuestionAnswer,
    ) -> Result<(), CommandRefusal> {
        message_service::send_user_message(
            &self.0,
            &Self::sender(viewer),
            &answer.task_id,
            message_service::UserMessage {
                resume: true,
                tool_response: Some(message_service::ToolResponse {
                    tool_use_id: answer.tool_use_id,
                    tool_name: answer.tool_name,
                    payload: answer.payload,
                }),
                ..Default::default()
            },
        )
        .await
        .map(|_| ())
        .map_err(|e| send_refusal(&answer.task_id, e))
    }
}

/// The agent a thread created from T3 runs.
fn default_agent_for(instance: &str) -> crate::shared_models::AgentType {
    let id = crate::agent::catalog::catalog()
        .iter()
        .find(|entry| super::providers::instance_for(entry.kind) == instance)
        .map_or(crate::shared_models::AgentType::CLAUDE_CODE, |entry| {
            entry.id
        });
    crate::shared_models::AgentType(id.to_string())
}

fn send_refusal(task_id: &str, error: MessageError) -> CommandRefusal {
    match error {
        MessageError::NotFound(_) => CommandRefusal::NotWritable {
            task_id: task_id.to_string(),
        },
        MessageError::TaskClosed(_) => CommandRefusal::Rejected {
            message: "This task is closed and cannot take new messages.".to_string(),
        },
        MessageError::BadRequest(message) => CommandRefusal::Rejected { message },
        other => {
            tracing::warn!(error = %other, task_id, "T3 send failed");
            CommandRefusal::Failed {
                message: "The message could not be sent.".to_string(),
            }
        }
    }
}

fn create_refusal(task_id: &str, error: TaskError) -> CommandRefusal {
    match error {
        TaskError::NotFound(_) | TaskError::Forbidden(_) => CommandRefusal::NotWritable {
            task_id: task_id.to_string(),
        },
        TaskError::BadRequest(message) => CommandRefusal::Rejected { message },
        TaskError::RepoNotFound(id) => CommandRefusal::Rejected {
            message: format!("Repository {id} could not be found."),
        },
        other => {
            tracing::warn!(error = %other, task_id, "T3 task command failed");
            CommandRefusal::Failed {
                message: "The action could not be completed.".to_string(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::catalog;

    /// The instance a person picks in T3 is what decides which agent runs the
    /// thread. Getting this backwards creates a Claude Task for somebody who
    /// asked for Codex, and the harness cannot be changed afterwards.
    #[test]
    fn each_instance_maps_to_the_first_catalog_agent_of_its_kind() {
        assert_eq!(
            default_agent_for(super::super::providers::CLAUDE_INSTANCE),
            crate::shared_models::AgentType(catalog::CLAUDE_CODE.id.to_string())
        );
        assert_eq!(
            default_agent_for(super::super::providers::CODEX_INSTANCE),
            crate::shared_models::AgentType(catalog::CODEX.id.to_string())
        );
    }

    /// An instance this server never published cannot arrive from a client that
    /// read our provider list, so it is a bug rather than a choice. Claude is
    /// the safe floor: it is the deployment's default harness.
    #[test]
    fn an_unknown_instance_falls_back_to_claude() {
        assert_eq!(
            default_agent_for("somethingElse"),
            crate::shared_models::AgentType(
                crate::shared_models::AgentType::CLAUDE_CODE.to_string()
            )
        );
    }
}
