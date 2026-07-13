//! gRPC server implementing `SandboxService`.
//!
//! Receives commands from the backend (execute agent, stop agent, notify
//! messages, status checks, file search). Server management is handled
//! by K8s (separate containers), not by the sandbox.

use tonic::{Request, Response, Status};
use tracing::info;

use sandbox_proto::sandbox_service_server::SandboxService;
use sandbox_proto::{
    scheduled_job_info, ExecuteAgentRequest, ExecuteAgentResponse, GetAgentStatusRequest,
    GetAgentStatusResponse, GetFileTreeRequest, GetFileTreeResponse, ListFilesRequest,
    ListFilesResponse, NotifyMessagesRequest, NotifyMessagesResponse, ReadFileRequest,
    ReadFileResponse, RefreshCredentialRequest, RefreshCredentialResponse, ScheduledJobInfo,
    SearchFilesByNameRequest, SearchFilesByNameResponse, StopAgentRequest, StopAgentResponse,
    WakeupJobInfo, WriteFileRequest, WriteFileResponse,
};

use moatless_events::AgentSubType;

use crate::agent::scheduled_job::JobSnapshot;
use crate::agent::types::AgentStatus;
use crate::files;
use crate::state::SandboxState;

/// gRPC server for sandbox commands.
pub struct SandboxServiceImpl {
    pub state: SandboxState,
}

#[tonic::async_trait]
impl SandboxService for SandboxServiceImpl {
    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn execute_agent(
        &self,
        _request: Request<ExecuteAgentRequest>,
    ) -> Result<Response<ExecuteAgentResponse>, Status> {
        // Fetch pending messages and send to the agent actor.
        // The actor handles whether to start a new session or inject into a running one.
        let state = self.state.clone();
        tokio::spawn(async move {
            crate::agent::dispatcher::dispatch_pending_messages(state).await;
        });

        info!("Agent execution triggered via gRPC");
        Ok(Response::new(ExecuteAgentResponse {}))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn stop_agent(
        &self,
        _request: Request<StopAgentRequest>,
    ) -> Result<Response<StopAgentResponse>, Status> {
        self.state
            .agent
            .stop()
            .await
            .map_err(|e| Status::failed_precondition(e.to_string()))?;
        self.state.agent.set_stopped().await;

        // Publish agent.stopped event so backend can emit taskTurn.stopped
        self.state
            .events
            .publish_agent_event(
                &self.state.config.task_id,
                AgentSubType::Stopped,
                serde_json::json!({}),
            )
            .await;

        info!("Agent stopped via gRPC");
        Ok(Response::new(StopAgentResponse {}))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn get_agent_status(
        &self,
        _request: Request<GetAgentStatusRequest>,
    ) -> Result<Response<GetAgentStatusResponse>, Status> {
        let status = match self.state.agent.status().await {
            AgentStatus::Idle => "idle",
            AgentStatus::Running => "running",
            AgentStatus::Stopped => "stopped",
        };

        let scheduled_jobs = self
            .state
            .scheduled_jobs
            .list()
            .await
            .into_iter()
            .map(|snap| match snap {
                JobSnapshot::Wakeup(w) => ScheduledJobInfo {
                    id: w.id,
                    registered_at_unix_ms: w.registered_at_unix_ms,
                    kind: Some(scheduled_job_info::Kind::Wakeup(WakeupJobInfo {
                        reason: w.reason,
                        prompt_preview: w.prompt_preview,
                        wakeup_at_unix_ms: w.wakeup_at_unix_ms,
                    })),
                },
            })
            .collect();

        // In tmux mode the persistent session keeps the CLI (and tmux server)
        // alive as descendants even at idle, which would read as always-busy —
        // report false there, same as older sandbox images.
        let busy_processes = if self.state.config.use_tmux {
            false
        } else {
            tokio::task::spawn_blocking(|| {
                crate::process_tree::has_live_descendants(std::process::id())
            })
            .await
            .unwrap_or(false)
        };

        Ok(Response::new(GetAgentStatusResponse {
            status: status.to_string(),
            session_id: self.state.agent.last_session_id().await,
            last_error: self.state.agent.last_error().await,
            scheduled_jobs,
            busy_processes,
        }))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn notify_messages(
        &self,
        _request: Request<NotifyMessagesRequest>,
    ) -> Result<Response<NotifyMessagesResponse>, Status> {
        let state = self.state.clone();
        tokio::spawn(async move {
            crate::agent::dispatcher::dispatch_pending_messages(state).await;
        });
        info!("Message processing triggered via gRPC");
        Ok(Response::new(NotifyMessagesResponse {}))
    }

    /// Apply a backend-pushed credential refresh. Routes the value through
    /// the sandbox-wide [`Dispatcher`] which writes files / updates env / etc.
    /// per the sink registered for the credential key.
    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn refresh_credential(
        &self,
        request: Request<RefreshCredentialRequest>,
    ) -> Result<Response<RefreshCredentialResponse>, Status> {
        let req = request.into_inner();
        info!(task_id = %req.task_id, key = %req.key, "RefreshCredential received");

        match self
            .state
            .credential_dispatcher
            .dispatch(&req.key, &req.value)
            .await
        {
            Ok(true) => Ok(Response::new(RefreshCredentialResponse {})),
            Ok(false) => Err(Status::invalid_argument(format!(
                "No sink registered for credential key: {}",
                req.key
            ))),
            Err(e) => Err(Status::internal(format!("Credential dispatch failed: {e}"))),
        }
    }

    // ── File operations ──────────────────────────────────────────
    //
    // The file browser API is rooted at the workspace directory (`WORKSPACE_DIR`,
    // e.g. `/opt/moatless/workspace`) rather than the primary repo subdirectory,
    // so the frontend exposes the entire workspace. Git change overlays are
    // computed relative to the same root and degrade gracefully (no badges) when
    // the workspace root is not itself a git repository.

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn get_file_tree(
        &self,
        request: Request<GetFileTreeRequest>,
    ) -> Result<Response<GetFileTreeResponse>, Status> {
        let req = request.into_inner();
        let root_dir = &self.state.config.workspace_dir;
        let root_path = if req.root_path.is_empty() {
            "."
        } else {
            &req.root_path
        };

        let (tree, summary) =
            files::tree::get_file_tree_with_changes(root_dir, root_path, req.recursive)
                .await
                .map_err(|e| Status::internal(format!("Failed to build file tree: {e}")))?;

        Ok(Response::new(GetFileTreeResponse {
            tree: tree.into_iter().map(tree_node_to_proto).collect(),
            git_summary: Some(sandbox_proto::GitSummary {
                total_files: summary.total_files,
                total_additions: summary.total_additions,
                total_deletions: summary.total_deletions,
                total_staged: summary.total_staged,
                total_modified: summary.total_modified,
                compare_base: summary.compare_base,
                has_changes: summary.has_changes,
            }),
        }))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn read_file(
        &self,
        request: Request<ReadFileRequest>,
    ) -> Result<Response<ReadFileResponse>, Status> {
        let req = request.into_inner();
        let root_dir = &self.state.config.workspace_dir;

        if req.include_changes {
            let result = files::read_write::read_file_with_changes(root_dir, &req.path)
                .await
                .map_err(file_error_to_status)?;
            Ok(Response::new(ReadFileResponse {
                content: result.content,
                old_content: result.old_content,
                additions: result.additions,
                deletions: result.deletions,
                is_modified: result.is_modified,
                is_binary: if result.is_binary { Some(true) } else { None },
                media_type: result.media_type,
            }))
        } else {
            let result = files::read_write::read_file(root_dir, &req.path)
                .await
                .map_err(file_error_to_status)?;
            Ok(Response::new(ReadFileResponse {
                content: result.content,
                old_content: None,
                additions: None,
                deletions: None,
                is_modified: None,
                is_binary: if result.is_binary { Some(true) } else { None },
                media_type: result.media_type,
            }))
        }
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn write_file(
        &self,
        request: Request<WriteFileRequest>,
    ) -> Result<Response<WriteFileResponse>, Status> {
        let req = request.into_inner();
        let root_dir = &self.state.config.workspace_dir;

        files::read_write::write_file(root_dir, &req.path, &req.content)
            .await
            .map_err(file_error_to_status)?;

        info!(path = %req.path, "File written via gRPC");
        Ok(Response::new(WriteFileResponse {}))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn list_files(
        &self,
        request: Request<ListFilesRequest>,
    ) -> Result<Response<ListFilesResponse>, Status> {
        let req = request.into_inner();
        let root_dir = &self.state.config.workspace_dir;
        let path = if req.path.is_empty() { "." } else { &req.path };

        let files = files::tree::list_files(root_dir, path, req.recursive)
            .await
            .map_err(|e| Status::internal(format!("Failed to list files: {e}")))?;

        Ok(Response::new(ListFilesResponse { files }))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn search_files_by_name(
        &self,
        request: Request<SearchFilesByNameRequest>,
    ) -> Result<Response<SearchFilesByNameResponse>, Status> {
        let req = request.into_inner();
        let root_dir = &self.state.config.workspace_dir;

        let results = files::search::search_files_by_name(
            root_dir,
            &req.query,
            req.case_sensitive,
            req.max_results,
        )
        .await
        .map_err(|e| Status::internal(format!("Failed to search files: {e}")))?;

        let total_matches = results.len() as i32;
        Ok(Response::new(SearchFilesByNameResponse {
            files: results
                .into_iter()
                .map(|m| sandbox_proto::FileNameMatch {
                    path: m.path,
                    matches: m.matches,
                })
                .collect(),
            total_matches,
        }))
    }
}

// ── Helpers ──────────────────────────────────────────────────────

fn tree_node_to_proto(node: files::tree::FileTreeNode) -> sandbox_proto::FileTreeNode {
    sandbox_proto::FileTreeNode {
        name: node.name,
        path: node.path,
        node_type: match node.node_type {
            files::tree::FileNodeType::File => sandbox_proto::FileNodeType::File.into(),
            files::tree::FileNodeType::Directory => sandbox_proto::FileNodeType::Directory.into(),
        },
        is_changed: node.is_changed,
        git_status: node.git_status,
        additions: node.additions,
        deletions: node.deletions,
        is_staged: node.is_staged,
        children: node.children.into_iter().map(tree_node_to_proto).collect(),
    }
}

fn file_error_to_status(e: files::FileError) -> Status {
    match e {
        files::FileError::InvalidPath(msg) => Status::invalid_argument(msg),
        files::FileError::NotFound(msg) => Status::not_found(msg),
        files::FileError::IsDirectory(msg) => {
            Status::failed_precondition(format!("Path is a directory: {msg}"))
        }
        files::FileError::Io(e) => Status::internal(format!("IO error: {e}")),
    }
}
