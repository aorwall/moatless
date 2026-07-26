//! gRPC server implementing `SandboxService`.
//!
//! Receives commands from the backend (execute agent, stop agent, notify
//! messages, status checks, file search). Server management is handled
//! by K8s (separate containers), not by the sandbox.

use tonic::{Request, Response, Status};
use tracing::info;

use sandbox_proto::sandbox_service_server::SandboxService;
use sandbox_proto::{
    scheduled_job_info, BackgroundProcess, BackgroundWatchJobInfo, ExecuteAgentRequest,
    ExecuteAgentResponse, GetAgentStatusRequest, GetAgentStatusResponse, GetFileTreeRequest,
    GetFileTreeResponse, ListFilesRequest, ListFilesResponse, NotifyMessagesRequest,
    NotifyMessagesResponse, ReadFileRequest, ReadFileResponse, RefreshCredentialRequest,
    RefreshCredentialResponse, ScheduledJobInfo, SearchFilesByNameRequest,
    SearchFilesByNameResponse, StopAgentRequest, StopAgentResponse, WakeupJobInfo,
    WriteFileRequest, WriteFileResponse,
};

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
        //
        // Detached, but through the dispatcher's supervision seam: the RPC
        // returns before dispatch finishes (that is the contract — `Ok` means
        // "triggered"), and dispatch must outlive the call to hold the
        // single-flight guard across its ack retries. `spawn_supervised` changes
        // nothing about that lifetime; it only stops a panic from disappearing.
        let state = self.state.clone();
        crate::agent::dispatcher::spawn_supervised("execute-agent-dispatch", async move {
            crate::agent::dispatcher::dispatch_pending_messages(state).await;
        });

        info!("Agent execution triggered via gRPC");
        Ok(Response::new(ExecuteAgentResponse {}))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn stop_agent(
        &self,
        request: Request<StopAgentRequest>,
    ) -> Result<Response<StopAgentResponse>, Status> {
        // Route the stop through the lifecycle owner (CoordinatorService for the
        // harness path, legacy actor for `agent_type = None`) — never publishing
        // the terminal event here, and never branching on harness selection. The
        // ack resolves once exactly one `agent.stopped` has been published and the
        // Turn closed, so returning Ok means the stop was processed, not queued.
        let source = crate::agent::types::StopSource::from(request.get_ref().source());
        let ack_rx = self.state.agent_handle.stop(source);
        match tokio::time::timeout(std::time::Duration::from_secs(30), ack_rx).await {
            Ok(Ok(())) => {}
            Ok(Err(_)) => {
                // The lifecycle owner dropped the ack (e.g. the loop exited
                // without acking). The stop still took effect; log and continue.
                tracing::warn!(
                    task_id = %self.state.config.task_id,
                    "stop ack sender dropped before delivery"
                );
            }
            Err(_elapsed) => {
                tracing::warn!(
                    task_id = %self.state.config.task_id,
                    "stop ack timed out after 30s"
                );
                return Err(Status::deadline_exceeded("agent stop timed out"));
            }
        }

        info!("Agent stopped via gRPC");
        Ok(Response::new(StopAgentResponse {}))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn get_agent_status(
        &self,
        _request: Request<GetAgentStatusRequest>,
    ) -> Result<Response<GetAgentStatusResponse>, Status> {
        let agent_status = self.state.agent.status().await;
        let status = match agent_status {
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
                JobSnapshot::BackgroundWatch(b) => ScheduledJobInfo {
                    id: b.id,
                    registered_at_unix_ms: b.registered_at_unix_ms,
                    kind: Some(scheduled_job_info::Kind::BackgroundWatch(
                        BackgroundWatchJobInfo {
                            process_count: b.process_count,
                            commands: b.commands,
                        },
                    )),
                },
            })
            .collect();

        // Only enumerate live descendants when the agent is idle/stopped. While
        // a turn is running the sandbox's own `claude -p` subprocess (and its
        // transient tool/subagent children) are live descendants, so reporting
        // them would surface the agent's own turn as a phantom "background
        // process". The signal is only meaningful once the agent is no longer
        // running, where any live descendant is genuine detached background
        // work. In tmux mode the persistent session keeps the CLI + tmux server
        // alive even at idle, which would read as always-busy, so report empty
        // there as before.
        let tmux_pane_pid = self.state.agent.tmux_pane_pid().await;
        let background_processes: Vec<BackgroundProcess> = if should_report_background_processes(
            self.state.config.use_tmux,
            agent_status,
            tmux_pane_pid,
        ) {
            tokio::task::spawn_blocking(move || {
                let root = std::process::id();
                match tmux_pane_pid {
                    Some(pane) => {
                        crate::process_tree::live_descendants_excluding_session(root, pane)
                    }
                    None => crate::process_tree::live_descendants(root),
                }
            })
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|p| BackgroundProcess {
                pid: p.pid,
                command: p.command,
            })
            .collect()
        } else {
            Vec::new()
        };
        let busy_processes = !background_processes.is_empty();

        Ok(Response::new(GetAgentStatusResponse {
            status: status.to_string(),
            session_id: self.state.agent.last_session_id().await,
            last_error: self.state.agent.last_error().await,
            scheduled_jobs,
            busy_processes,
            background_processes,
        }))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn notify_messages(
        &self,
        _request: Request<NotifyMessagesRequest>,
    ) -> Result<Response<NotifyMessagesResponse>, Status> {
        // Same detached-but-supervised shape as `execute_agent` above.
        let state = self.state.clone();
        crate::agent::dispatcher::spawn_supervised("notify-messages-dispatch", async move {
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
    // computed per-repo: every git repository found within the workspace (the
    // root itself, or sibling repos in subdirectories) contributes changes,
    // with file paths prefixed by the repo's workspace-relative location so they
    // line up with the tree. Degrades gracefully (no badges) when no repos exist.

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

/// Whether `GetAgentStatus` should enumerate live descendant processes as
/// background work. Only meaningful while the agent is not running: mid-turn
/// the CLI's tool / sub-agent children are live descendants and would surface
/// as phantom "background processes"; once the agent is idle/stopped, any live
/// descendant is genuine background work.
///
/// In tmux mode the persistent session keeps the CLI and tmux server alive even
/// at idle, so a plain walk reads as always-busy. Reporting there needs the
/// pane PID, which identifies that spine so it can be excluded — without it,
/// report nothing rather than everything.
fn should_report_background_processes(
    use_tmux: bool,
    status: AgentStatus,
    tmux_pane_pid: Option<u32>,
) -> bool {
    if matches!(status, AgentStatus::Running) {
        return false;
    }
    !use_tmux || tmux_pane_pid.is_some()
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Both domain node types are representable on the wire, and the mapping is
    /// what the backend decodes back into `"file"` / `"directory"`. Nested nodes
    /// must carry their own discriminant, not the parent's.
    #[test]
    fn tree_node_types_map_onto_their_wire_discriminants() {
        fn node(
            name: &str,
            node_type: files::tree::FileNodeType,
            children: Vec<files::tree::FileTreeNode>,
        ) -> files::tree::FileTreeNode {
            files::tree::FileTreeNode {
                name: name.to_string(),
                path: name.to_string(),
                node_type,
                is_changed: false,
                git_status: None,
                additions: None,
                deletions: None,
                is_staged: None,
                children,
            }
        }

        let proto = tree_node_to_proto(node(
            "src",
            files::tree::FileNodeType::Directory,
            vec![node("main.rs", files::tree::FileNodeType::File, vec![])],
        ));

        assert_eq!(
            sandbox_proto::FileNodeType::try_from(proto.node_type),
            Ok(sandbox_proto::FileNodeType::Directory)
        );
        assert_eq!(
            sandbox_proto::FileNodeType::try_from(proto.children[0].node_type),
            Ok(sandbox_proto::FileNodeType::File)
        );
    }

    #[test]
    fn reports_background_processes_when_subprocess_and_not_running() {
        // Subprocess mode: idle/stopped surface genuine detached work...
        assert!(should_report_background_processes(
            false,
            AgentStatus::Idle,
            None
        ));
        assert!(should_report_background_processes(
            false,
            AgentStatus::Stopped,
            None
        ));
        // ...but a running turn's own CLI subprocess must not be reported.
        assert!(!should_report_background_processes(
            false,
            AgentStatus::Running,
            None
        ));
    }

    #[test]
    fn reports_background_processes_in_tmux_once_the_session_is_known() {
        // With the pane PID the persistent spine can be excluded, so idle work
        // is reportable and a running turn's tool children still are not.
        assert!(should_report_background_processes(
            true,
            AgentStatus::Idle,
            Some(61)
        ));
        assert!(should_report_background_processes(
            true,
            AgentStatus::Stopped,
            Some(61)
        ));
        assert!(!should_report_background_processes(
            true,
            AgentStatus::Running,
            Some(61)
        ));
    }

    #[test]
    fn tmux_without_a_pane_pid_reports_nothing() {
        // No way to tell the always-alive CLI + tmux server from real work, so
        // report none rather than pinning the sandbox as permanently busy.
        assert!(!should_report_background_processes(
            true,
            AgentStatus::Idle,
            None
        ));
        assert!(!should_report_background_processes(
            true,
            AgentStatus::Stopped,
            None
        ));
    }
}
