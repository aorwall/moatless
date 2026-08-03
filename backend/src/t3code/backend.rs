//! The Moatless side of the compatibility module's write ports.
//!
//! One implementation each of [`AgentCatalogSource`] and [`ThreadCommandSink`],
//! over [`AppState`]. Everything here is translation: a T3 command has already
//! been validated by `super::commands` by the time it arrives, and each method
//! turns it into the same service call the Moatless UI makes.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use moatless_api_schemas::agent::AgentDescriptor;
use moatless_backend_core::{sandbox as core_sandbox, scope::ScopeContext};

use crate::message::error::MessageError;
use crate::message::service as message_service;
use crate::repository::error::RepositoryError;
use crate::repository::handlers::list_branches;
use crate::sandbox::access as backend_sandbox;
use crate::sandbox::docker::provider::DockerSandboxAdapter;
use crate::sandbox::k8s::provider::K8sSandboxAdapter;
use crate::sandbox::lifecycle::Lifecycle;
use crate::state::AppState;
use crate::task::dao::TaskPatch;
use crate::task::error::TaskError;
use crate::task::handlers as task_handlers;
use crate::task::models::TaskStatus;
use crate::task::service as task_service;
use moatless_t3code::ports::{
    AgentCatalogSource, BranchListError, CommandRefusal, NewThread, QuestionAnswer,
    RepositoryBranchSource, ThreadCommandSink, ThreadMeta, ThreadTurn,
};
use moatless_t3code::{
    ClusterDiagnostics, SandboxConnector, SandboxIntentReader, SandboxObserver, SandboxStarter,
    SandboxStopper,
};

#[async_trait]
impl SandboxConnector for DockerSandboxAdapter {
    async fn sandbox_client(
        &self,
        task_id: &str,
    ) -> Result<core_sandbox::SandboxClient, core_sandbox::SandboxError> {
        backend_sandbox::SandboxConnector::sandbox_client(self, task_id)
            .await
            .map_err(sandbox_error)
    }
}

#[async_trait]
impl SandboxConnector for K8sSandboxAdapter {
    async fn sandbox_client(
        &self,
        task_id: &str,
    ) -> Result<core_sandbox::SandboxClient, core_sandbox::SandboxError> {
        backend_sandbox::SandboxConnector::sandbox_client(self, task_id)
            .await
            .map_err(sandbox_error)
    }
}

#[async_trait]
impl SandboxObserver for DockerSandboxAdapter {
    async fn sandbox_status(
        &self,
        task_id: &str,
        task_status: moatless_api_schemas::enums::TaskStatus,
        desired_state: moatless_api_schemas::enums::SandboxDesiredState,
    ) -> Result<core_sandbox::SandboxStatusFullResponse, core_sandbox::SandboxError> {
        backend_sandbox::SandboxObserver::sandbox_status(self, task_id, task_status, desired_state)
            .await
            .map(sandbox_status)
            .map_err(sandbox_error)
    }

    async fn live_status(
        &self,
        task_id: &str,
        task_status: moatless_api_schemas::enums::TaskStatus,
        desired_state: moatless_api_schemas::enums::SandboxDesiredState,
    ) -> Result<core_sandbox::LiveStatus, core_sandbox::SandboxError> {
        backend_sandbox::SandboxObserver::live_status(self, task_id, task_status, desired_state)
            .await
            .map(live_status)
            .map_err(sandbox_error)
    }

    async fn server_logs(
        &self,
        task_id: &str,
        server_name: &str,
        options: core_sandbox::LogOptions,
    ) -> Result<core_sandbox::ServerLogStream, core_sandbox::SandboxError> {
        backend_sandbox::SandboxObserver::server_logs(
            self,
            task_id,
            server_name,
            log_options(options),
        )
        .await
        .map(|stream| stream.map(|item| item.map_err(sandbox_error)).boxed())
        .map_err(sandbox_error)
    }
}

#[async_trait]
impl SandboxObserver for K8sSandboxAdapter {
    async fn sandbox_status(
        &self,
        task_id: &str,
        task_status: moatless_api_schemas::enums::TaskStatus,
        desired_state: moatless_api_schemas::enums::SandboxDesiredState,
    ) -> Result<core_sandbox::SandboxStatusFullResponse, core_sandbox::SandboxError> {
        backend_sandbox::SandboxObserver::sandbox_status(self, task_id, task_status, desired_state)
            .await
            .map(sandbox_status)
            .map_err(sandbox_error)
    }

    async fn live_status(
        &self,
        task_id: &str,
        task_status: moatless_api_schemas::enums::TaskStatus,
        desired_state: moatless_api_schemas::enums::SandboxDesiredState,
    ) -> Result<core_sandbox::LiveStatus, core_sandbox::SandboxError> {
        backend_sandbox::SandboxObserver::live_status(self, task_id, task_status, desired_state)
            .await
            .map(live_status)
            .map_err(sandbox_error)
    }

    async fn server_logs(
        &self,
        task_id: &str,
        server_name: &str,
        options: core_sandbox::LogOptions,
    ) -> Result<core_sandbox::ServerLogStream, core_sandbox::SandboxError> {
        backend_sandbox::SandboxObserver::server_logs(
            self,
            task_id,
            server_name,
            log_options(options),
        )
        .await
        .map(|stream| stream.map(|item| item.map_err(sandbox_error)).boxed())
        .map_err(sandbox_error)
    }
}

#[async_trait]
impl ClusterDiagnostics for DockerSandboxAdapter {
    /// Local Docker has no fleet-wide view — a container is only ever addressed
    /// by its Task. Empty rather than an error, so the diagnostics page renders
    /// normally under `docker-compose`.
    async fn list_sandbox_diagnostics(
        &self,
    ) -> Result<Vec<core_sandbox::SandboxDiagnosticsEntry>, core_sandbox::SandboxError> {
        Ok(Vec::new())
    }
}

#[async_trait]
impl ClusterDiagnostics for K8sSandboxAdapter {
    async fn list_sandbox_diagnostics(
        &self,
    ) -> Result<Vec<core_sandbox::SandboxDiagnosticsEntry>, core_sandbox::SandboxError> {
        crate::sandbox::k8s::pod_reader::fetch_fleet_diagnostics(&self.k8s)
            .await
            .map_err(|e| core_sandbox::SandboxError::Other(e.to_string()))
    }
}

#[async_trait]
impl SandboxIntentReader for Lifecycle {
    async fn desired_state(
        &self,
        task_id: &str,
    ) -> Result<moatless_api_schemas::enums::SandboxDesiredState, core_sandbox::SandboxError> {
        backend_sandbox::SandboxIntentReader::desired_state(self, task_id)
            .await
            .map_err(sandbox_error)
    }
}

#[async_trait]
impl SandboxStarter for Lifecycle {
    async fn start(&self, task_id: &str) -> Result<(), core_sandbox::SandboxError> {
        backend_sandbox::SandboxStarter::start(self, task_id)
            .await
            .map_err(sandbox_error)
    }
}

#[async_trait]
impl SandboxStopper for Lifecycle {
    async fn stop(
        &self,
        task_id: &str,
        reason: moatless_api_schemas::enums::SandboxStopReason,
    ) -> Result<(), core_sandbox::SandboxError> {
        backend_sandbox::SandboxStopper::stop(self, task_id, reason)
            .await
            .map_err(sandbox_error)
    }
}

fn sandbox_status(
    response: crate::sandbox::schemas::SandboxStatusFullResponse,
) -> core_sandbox::SandboxStatusFullResponse {
    core_sandbox::SandboxStatusFullResponse {
        sandbox_status: response.sandbox_status,
        servers: response.servers.into_iter().map(server_entry).collect(),
    }
}

fn live_status(status: crate::sandbox::live_status::LiveStatus) -> core_sandbox::LiveStatus {
    core_sandbox::LiveStatus {
        sandbox_status: status.sandbox_status,
        agent_status: status.agent_status.map(|status| match status {
            crate::sandbox::live_status::AgentStatus::Running => core_sandbox::AgentStatus::Running,
            crate::sandbox::live_status::AgentStatus::Waiting => core_sandbox::AgentStatus::Waiting,
            crate::sandbox::live_status::AgentStatus::Idle => core_sandbox::AgentStatus::Idle,
            crate::sandbox::live_status::AgentStatus::Unknown => core_sandbox::AgentStatus::Unknown,
        }),
        sandbox_error: status.sandbox_error,
    }
}

fn server_entry(entry: crate::sandbox::schemas::ServerEntry) -> core_sandbox::ServerEntry {
    core_sandbox::ServerEntry {
        name: entry.name,
        label: entry.label,
        port: entry.port,
        status: match entry.status {
            crate::sandbox::models::ServerStatus::Stopped => core_sandbox::ServerStatus::Stopped,
            crate::sandbox::models::ServerStatus::Installing => {
                core_sandbox::ServerStatus::Installing
            }
            crate::sandbox::models::ServerStatus::Starting => core_sandbox::ServerStatus::Starting,
            crate::sandbox::models::ServerStatus::Started => core_sandbox::ServerStatus::Started,
            crate::sandbox::models::ServerStatus::Failed => core_sandbox::ServerStatus::Failed,
        },
        url: entry.url,
        error: entry.error,
        detail: entry.detail,
        default: entry.default,
        capabilities: core_sandbox::ServerCapabilities {
            address_bar: entry.capabilities.address_bar,
            inspector: entry.capabilities.inspector,
            reload: entry.capabilities.reload,
            open_external: entry.capabilities.open_external,
            viewport: entry.capabilities.viewport,
            console: entry.capabilities.console,
        },
    }
}

fn log_options(options: core_sandbox::LogOptions) -> backend_sandbox::LogOptions {
    backend_sandbox::LogOptions {
        previous: options.previous,
        follow: options.follow,
        tail_lines: options.tail_lines,
    }
}

fn sandbox_error(error: crate::sandbox::error::SandboxError) -> core_sandbox::SandboxError {
    use crate::sandbox::error::SandboxError;
    match error {
        SandboxError::NotReady(message) => core_sandbox::SandboxError::NotReady(message),
        SandboxError::NotFound(message) => core_sandbox::SandboxError::NotFound(message),
        SandboxError::BadRequest(message) => core_sandbox::SandboxError::BadRequest(message),
        SandboxError::Unsupported { operation, source } => {
            core_sandbox::SandboxError::Unsupported { operation, source }
        }
        SandboxError::Grpc { operation, source } => {
            core_sandbox::SandboxError::Grpc { operation, source }
        }
        SandboxError::GrpcClient(source) => {
            core_sandbox::SandboxError::GrpcClient(source.to_string())
        }
        other => core_sandbox::SandboxError::Other(other.to_string()),
    }
}

pub struct AppStateAgents(pub AppState);

#[async_trait]
impl AgentCatalogSource for AppStateAgents {
    async fn list(&self, viewer: &ScopeContext) -> Vec<AgentDescriptor> {
        crate::agent::service::list_agents(&self.0, viewer).await
    }
}

pub struct AppStateBranches(pub AppState);

#[async_trait]
impl RepositoryBranchSource for AppStateBranches {
    async fn list_branches(
        &self,
        viewer: &ScopeContext,
        repository_id: &str,
    ) -> Result<moatless_api_schemas::repository::BranchListResponse, BranchListError> {
        list_branches(&self.0, viewer, repository_id)
            .await
            .map_err(branch_list_error)
    }
}

/// A repository read failure, as the branch picker is told it.
///
/// A repository the viewer cannot read answers `NotFound`, the same as one that
/// is gone — neither is a branch list to show. A missing token or a git host
/// that refused is surfaced verbatim; a database failure is kept apart so it is
/// logged rather than sent.
fn branch_list_error(error: RepositoryError) -> BranchListError {
    match error {
        RepositoryError::NotFound(_) | RepositoryError::Forbidden(_) => BranchListError::None,
        RepositoryError::BadRequest(message)
        | RepositoryError::ProviderError(message)
        | RepositoryError::Conflict(message) => BranchListError::Unavailable(message),
        RepositoryError::Db(error) => BranchListError::Failed(error),
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
                effort: turn.effort,
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
            branch: thread.branch,
            message: thread.first_message.as_ref().map(|m| m.text.clone()),
            model: thread.model,
            agent_mode: thread
                .first_message
                .as_ref()
                .and_then(|m| m.agent_mode.clone()),
            effort: thread.first_message.as_ref().and_then(|m| m.effort.clone()),
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

    async fn archive(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        task_handlers::close_task_for(&self.0, viewer, task_id)
            .await
            .map_err(|e| create_refusal(task_id, e))
    }

    async fn unarchive(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        task_handlers::reopen_task_for(&self.0, viewer, task_id)
            .await
            .map_err(|e| create_refusal(task_id, e))
    }

    async fn settle(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.ensure_followed_open(viewer, task_id).await?;
        self.0
            .tasks
            .settle_task(&viewer.user_id, task_id, chrono::Utc::now())
            .await
            .map_err(|e| Self::state_refusal(task_id, e))?;
        // Settling says nobody is waiting on this thread any more, so the
        // sandbox it was holding is released rather than left for the idle
        // reaper. Best-effort: the settlement is already durable, and a viewer
        // who follows a thread they may not control has no sandbox to stop —
        // neither case is a reason to report the settle as refused. New
        // activity clears the settlement (`clear_settled_for_all_followers`)
        // and the next turn starts the sandbox again.
        if let Err(error) = task_service::stop_task(&self.0, viewer, task_id).await {
            tracing::warn!(%error, task_id, "Sandbox stop on settle failed");
        }
        Ok(())
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
        .find(|entry| agent_instance_for(entry.kind) == instance)
        .map_or(crate::shared_models::AgentType::CLAUDE_CODE, |entry| {
            entry.id
        });
    crate::shared_models::AgentType(id.to_string())
}

fn agent_instance_for(kind: crate::agent::catalog::AgentKind) -> &'static str {
    match kind {
        crate::agent::catalog::AgentKind::ClaudeCode => "claudeAgent",
        crate::agent::catalog::AgentKind::Codex => "codex",
    }
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
            default_agent_for("claudeAgent"),
            crate::shared_models::AgentType(catalog::CLAUDE_CODE.id.to_string())
        );
        assert_eq!(
            default_agent_for("codex"),
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
