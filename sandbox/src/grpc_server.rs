//! gRPC server implementing `SandboxService`.
//!
//! Receives commands from the backend (execute agent, stop agent, notify
//! messages, status checks, file search). Preview servers still run as
//! separate containers owned by K8s/Docker — the sandbox does not supervise
//! them — but it does own their live override files (`ApplyServerConfig`),
//! which is what lets a server be reconfigured without rebuilding the pod.

use std::pin::Pin;

use futures::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tonic::{Request, Response, Status};
use tracing::info;

use sandbox_proto::sandbox_service_server::SandboxService;
use sandbox_proto::{
    ApplyServerConfigRequest, ApplyServerConfigResponse, BackgroundProcess, BackgroundWatchJobInfo,
    ExecuteAgentRequest, ExecuteAgentResponse, GetAgentStatusRequest, GetAgentStatusResponse,
    GetFileTreeRequest, GetFileTreeResponse, GetGitDiffPreviewRequest, GetGitDiffPreviewResponse,
    GetGitStatusRequest, GetGitStatusResponse, GetServerConfigRequest, GetServerConfigResponse,
    GitActionEvent, GitRef, GitRepoRange, GitStackedAction, GitWorkingTreeFile, ListFilesRequest,
    ListFilesResponse, ListGitRefsRequest, ListGitRefsResponse, NotifyMessagesRequest,
    NotifyMessagesResponse, ReadFileRequest, ReadFileResponse, RefreshCredentialRequest,
    RefreshCredentialResponse, RunGitStackedActionRequest, ScheduledJobInfo,
    SearchFilesByNameRequest, SearchFilesByNameResponse, ServerConfigOverride, StopAgentRequest,
    StopAgentResponse, TerminalAttachRequest, TerminalAttachStreamEvent, TerminalClearRequest,
    TerminalClearResponse, TerminalCloseRequest, TerminalCloseResponse, TerminalOpenRequest,
    TerminalResizeRequest, TerminalResizeResponse, TerminalRestartRequest, TerminalSessionSnapshot,
    TerminalWriteRequest, TerminalWriteResponse, WakeupJobInfo, WriteFileRequest,
    WriteFileResponse, scheduled_job_info,
};

use crate::agent::scheduled_job::JobSnapshot;
use crate::agent::types::AgentStatus;
use crate::files;
use crate::git::actions::{self, ActionEnv, StackedActionParams};
use crate::server_control;
use crate::state::SandboxState;

/// gRPC server for sandbox commands.
pub struct SandboxServiceImpl {
    pub state: SandboxState,
}

#[tonic::async_trait]
impl SandboxService for SandboxServiceImpl {
    type TerminalAttachStream =
        Pin<Box<dyn Stream<Item = Result<TerminalAttachStreamEvent, Status>> + Send + 'static>>;
    type RunGitStackedActionStream =
        Pin<Box<dyn Stream<Item = Result<GitActionEvent, Status>> + Send + 'static>>;

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn execute_agent(
        &self,
        _request: Request<ExecuteAgentRequest>,
    ) -> Result<Response<ExecuteAgentResponse>, Status> {
        // Fetch pending messages and send to the agent lifecycle task.
        // The coordinator handles whether to start a new session or inject into a running one.
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
        // Route the stop through the lifecycle owner — never publishing the
        // terminal event here, and never branching on harness selection. The
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
            AgentStatus::Waiting => "waiting",
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

    /// Write a preview server's override into the workspace volume and
    /// optionally bump its generation so the sidecar re-execs in place.
    ///
    /// The request carries the complete override state rather than a delta, so
    /// this never merges. The control files it writes are the only record of an
    /// override; a caller that needs a partial update reads the current state
    /// with `GetServerConfig` first.
    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn apply_server_config(
        &self,
        request: Request<ApplyServerConfigRequest>,
    ) -> Result<Response<ApplyServerConfigResponse>, Status> {
        let req = request.into_inner();
        info!(
            server = %req.server_name,
            restart = req.restart,
            env_count = req.env.len(),
            "ApplyServerConfig received"
        );

        if !server_control::is_valid_server_name(&req.server_name) {
            return Err(Status::invalid_argument(format!(
                "Invalid server name: {}",
                req.server_name
            )));
        }

        let ov = server_control::ServerOverride {
            start_command: req.start_command,
            env: req.env.into_iter().collect(),
            unset_env: req.unset_env,
        };

        let generation = server_control::apply(
            &self.state.config.workspace_dir,
            &req.server_name,
            &ov,
            req.restart,
        )
        .await
        .map_err(|e| Status::internal(format!("Failed to apply server config: {e}")))?;

        Ok(Response::new(ApplyServerConfigResponse { generation }))
    }

    /// Read back every override currently on the workspace volume.
    ///
    /// The backend has no other source: it stopped persisting overrides once
    /// they became workspace state, so this call is the only way to answer
    /// "what is actually running". It therefore reports what is on disk, not
    /// what anyone intended — a hand-edited control file shows up here.
    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn get_server_config(
        &self,
        _request: Request<GetServerConfigRequest>,
    ) -> Result<Response<GetServerConfigResponse>, Status> {
        let overrides = server_control::read_all(&self.state.config.workspace_dir)
            .await
            .into_iter()
            .map(|(name, live)| {
                (
                    name,
                    ServerConfigOverride {
                        start_command: live.config.start_command,
                        env: live.config.env.into_iter().collect(),
                        unset_env: live.config.unset_env,
                        generation: live.generation,
                    },
                )
            })
            .collect();

        Ok(Response::new(GetServerConfigResponse { overrides }))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn terminal_open(
        &self,
        request: Request<TerminalOpenRequest>,
    ) -> Result<Response<TerminalSessionSnapshot>, Status> {
        let snapshot = self.state.terminals.open(request.into_inner()).await?;
        Ok(Response::new(snapshot))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn terminal_attach(
        &self,
        request: Request<TerminalAttachRequest>,
    ) -> Result<Response<Self::TerminalAttachStream>, Status> {
        let attached = self.state.terminals.attach(request.into_inner()).await?;
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let mut events = attached.events;
        let snapshot_event = TerminalAttachStreamEvent {
            thread_id: attached.snapshot.thread_id.clone(),
            terminal_id: attached.snapshot.terminal_id.clone(),
            sequence: attached.snapshot.sequence,
            event: Some(
                sandbox_proto::terminal_attach_stream_event::Event::Snapshot(attached.snapshot),
            ),
        };
        let _handle = tokio::spawn(async move {
            if tx.send(Ok(snapshot_event)).await.is_err() {
                return;
            }
            loop {
                match events.recv().await {
                    Ok(event) => {
                        if tx.send(Ok(event)).await.is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        Ok(Response::new(
            Box::pin(ReceiverStream::new(rx)) as Self::TerminalAttachStream
        ))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn terminal_write(
        &self,
        request: Request<TerminalWriteRequest>,
    ) -> Result<Response<TerminalWriteResponse>, Status> {
        self.state.terminals.write(request.into_inner()).await?;
        Ok(Response::new(TerminalWriteResponse {}))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn terminal_resize(
        &self,
        request: Request<TerminalResizeRequest>,
    ) -> Result<Response<TerminalResizeResponse>, Status> {
        self.state.terminals.resize(request.into_inner()).await?;
        Ok(Response::new(TerminalResizeResponse {}))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn terminal_clear(
        &self,
        request: Request<TerminalClearRequest>,
    ) -> Result<Response<TerminalClearResponse>, Status> {
        self.state.terminals.clear(request.into_inner()).await?;
        Ok(Response::new(TerminalClearResponse {}))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn terminal_restart(
        &self,
        request: Request<TerminalRestartRequest>,
    ) -> Result<Response<TerminalSessionSnapshot>, Status> {
        let snapshot = self.state.terminals.restart(request.into_inner()).await?;
        Ok(Response::new(snapshot))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn terminal_close(
        &self,
        request: Request<TerminalCloseRequest>,
    ) -> Result<Response<TerminalCloseResponse>, Status> {
        self.state.terminals.close(request.into_inner()).await?;
        Ok(Response::new(TerminalCloseResponse {}))
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

        let files = files::tree::list_files(root_dir, path, req.recursive, req.respect_gitignore)
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

    // ── Git review ───────────────────────────────────────────────
    //
    // Diffs span every repository in the workspace and carry
    // workspace-relative paths, so a file a patch names is a file `ReadFile`
    // can open. Branch state and refs answer for the primary repository, whose
    // HEAD and ref namespace are the only ones those questions have.

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn get_git_diff_preview(
        &self,
        request: Request<GetGitDiffPreviewRequest>,
    ) -> Result<Response<GetGitDiffPreviewResponse>, Status> {
        let req = request.into_inner();
        let preview = files::review::diff_preview(
            &self.state.config.workspace_dir,
            req.base_ref.as_deref(),
            req.ignore_whitespace,
        )
        .await;

        Ok(Response::new(GetGitDiffPreviewResponse {
            working_tree_diff: preview.working_tree.diff,
            working_tree_truncated: preview.working_tree.truncated,
            branch_diff: preview.branch.diff,
            branch_truncated: preview.branch.truncated,
            ranges: preview
                .ranges
                .into_iter()
                .map(|range| GitRepoRange {
                    prefix: range.prefix,
                    base_ref: range.base_ref.unwrap_or_default(),
                    head_ref: range.head_ref.unwrap_or_default(),
                })
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn get_git_status(
        &self,
        _request: Request<GetGitStatusRequest>,
    ) -> Result<Response<GetGitStatusResponse>, Status> {
        let status = files::review::status(
            &self.state.config.workspace_dir,
            &self.state.config.repo_dir,
        )
        .await;

        Ok(Response::new(GetGitStatusResponse {
            is_repo: status.is_repo,
            ref_name: status.ref_name.unwrap_or_default(),
            is_default_ref: status.is_default_ref,
            has_primary_remote: status.has_primary_remote,
            has_upstream: status.has_upstream,
            ahead_count: status.ahead_count,
            behind_count: status.behind_count,
            ahead_of_default_count: status.ahead_of_default_count,
            has_working_tree_changes: status.has_working_tree_changes,
            working_tree_files: status
                .working_tree_files
                .into_iter()
                .map(|file| GitWorkingTreeFile {
                    path: file.path,
                    insertions: file.insertions,
                    deletions: file.deletions,
                })
                .collect(),
        }))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn list_git_refs(
        &self,
        _request: Request<ListGitRefsRequest>,
    ) -> Result<Response<ListGitRefsResponse>, Status> {
        let listing = files::review::list_refs(&self.state.config.repo_dir).await;

        Ok(Response::new(ListGitRefsResponse {
            is_repo: listing.is_repo,
            has_primary_remote: listing.has_primary_remote,
            refs: listing
                .refs
                .into_iter()
                .map(|reference| GitRef {
                    name: reference.name,
                    is_remote: reference.is_remote,
                    remote_name: reference.remote_name.unwrap_or_default(),
                    current: reference.current,
                    is_default: reference.is_default,
                })
                .collect(),
            truncated: listing.truncated,
        }))
    }

    #[tracing::instrument(skip_all, fields(request_id = %uuid::Uuid::new_v4(), task_id = %self.state.config.task_id))]
    async fn run_git_stacked_action(
        &self,
        request: Request<RunGitStackedActionRequest>,
    ) -> Result<Response<Self::RunGitStackedActionStream>, Status> {
        let request = request.into_inner();
        let action = request.action();
        if action == GitStackedAction::Unspecified {
            return Err(Status::invalid_argument("action is required"));
        }

        // The primary checkout, or the named sibling repository under the
        // workspace when the wire selects one.
        let repo_dir = match &request.repository {
            Some(repository) if !repository.is_empty() => {
                format!("{}/{repository}", self.state.config.workspace_dir)
            }
            _ => self.state.config.repo_dir.clone(),
        };
        let env = ActionEnv {
            repo_dir,
            claude_cli_path: self.state.config.claude_cli_path.clone(),
            model: self.state.config.default_model.clone(),
        };
        let params = StackedActionParams {
            action,
            commit_message: request.commit_message,
            feature_branch: request.feature_branch,
            file_paths: request.file_paths,
        };

        let (tx, rx) = tokio::sync::mpsc::channel(64);
        tokio::spawn(actions::run_stacked_action(env, params, tx));
        Ok(Response::new(
            Box::pin(ReceiverStream::new(rx)) as Self::RunGitStackedActionStream
        ))
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
    // Waiting counts as running here: the CLI process and its children are
    // still alive, parked mid-turn on the answer.
    if matches!(status, AgentStatus::Running | AgentStatus::Waiting) {
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
