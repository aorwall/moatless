//! gRPC server implementing `SandboxService`.
//!
//! Receives commands from the backend (execute agent, stop agent, notify
//! messages, status checks, file search, server logs). Replaces the
//! HTTP endpoints on port 8001/8765.

use std::time::Duration;

use tonic::{Request, Response, Status};
use tracing::{info, warn};

use sandbox_proto::sandbox_service_server::SandboxService;
use sandbox_proto::{
    ExecuteAgentRequest, ExecuteAgentResponse, GetServerLogsRequest, GetServerStatusRequest,
    GetServerStatusResponse, GetStatusRequest, GetStatusResponse, HealthCheckRequest,
    HealthCheckResponse, LogLine, NotifyMessagesRequest, NotifyMessagesResponse,
    SearchFilesRequest, SearchFilesResponse, StopAgentRequest, StopAgentResponse,
};

use crate::agent::claude_code::spawn_agent_execution;
use crate::agent::types::{AgentRunRequest, AgentStatus, ContentBlock, ImageSource};
use crate::state::SandboxState;

/// gRPC server for sandbox commands.
pub struct SandboxServiceImpl {
    pub state: SandboxState,
}

#[tonic::async_trait]
impl SandboxService for SandboxServiceImpl {
    async fn execute_agent(
        &self,
        request: Request<ExecuteAgentRequest>,
    ) -> Result<Response<ExecuteAgentResponse>, Status> {
        let req = request.into_inner();

        // Check if agent is already running
        if self.state.agent.status().await == AgentStatus::Running {
            return Err(Status::already_exists("Agent is already running"));
        }

        // Convert proto ContentBlock to internal ContentBlock
        let content: Vec<ContentBlock> = req
            .content
            .into_iter()
            .filter_map(|block| {
                block.block.map(|b| match b {
                    sandbox_proto::content_block::Block::Text(t) => {
                        ContentBlock::Text { text: t.text }
                    }
                    sandbox_proto::content_block::Block::Image(img) => {
                        ContentBlock::Image {
                            source: ImageSource {
                                source_type: "url".to_string(),
                                media_type: img.media_type,
                                data: img.url,
                            },
                        }
                    }
                })
            })
            .collect();

        let content_blocks = content.len();
        let agent_request = AgentRunRequest {
            content,
            model: req.model.clone(),
            system_prompt: req.system_prompt,
            resume: req.resume.map(serde_json::Value::Bool),
            agent_type: req.agent_type.clone(),
        };

        spawn_agent_execution(self.state.clone(), agent_request);

        info!(
            blocks = content_blocks,
            model = ?req.model,
            agent_type = ?req.agent_type,
            "Agent execution started via gRPC"
        );
        Ok(Response::new(ExecuteAgentResponse {}))
    }

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

        info!("Agent stopped via gRPC");
        Ok(Response::new(StopAgentResponse {}))
    }

    async fn notify_messages(
        &self,
        _request: Request<NotifyMessagesRequest>,
    ) -> Result<Response<NotifyMessagesResponse>, Status> {
        self.state.consumer_notify.notify_one();
        info!("Consumer notified via gRPC");
        Ok(Response::new(NotifyMessagesResponse {}))
    }

    async fn get_status(
        &self,
        _request: Request<GetStatusRequest>,
    ) -> Result<Response<GetStatusResponse>, Status> {
        let agent_status = self.state.agent.status().await;
        let last_session_id = self.state.agent.last_session_id().await;
        let last_error = self.state.agent.last_error().await;
        let has_pending_prompt = self.state.agent.has_pending_prompt().await;

        let server_statuses = match tokio::time::timeout(
            Duration::from_secs(1),
            self.state.servers.get_all_statuses(),
        )
        .await
        {
            Ok(statuses) => statuses,
            Err(_) => {
                warn!("Server status collection timed out after 1s, returning empty list");
                vec![]
            }
        };

        let status_json = serde_json::json!({
            "taskId": self.state.config.task_id,
            "agent": {
                "state": agent_status,
                "hasAgent": agent_status != AgentStatus::Stopped && agent_status != AgentStatus::Idle,
                "error": last_error,
                "lastSessionId": last_session_id,
                "hasPendingPrompt": has_pending_prompt,
            },
            "servers": server_statuses,
        });

        Ok(Response::new(GetStatusResponse {
            status_json: status_json.to_string(),
        }))
    }

    async fn check_health(
        &self,
        _request: Request<HealthCheckRequest>,
    ) -> Result<Response<HealthCheckResponse>, Status> {
        let phase = self.state.init_phase.read().await;
        let healthy = matches!(*phase, crate::init::InitPhase::Ready);
        Ok(Response::new(HealthCheckResponse { healthy }))
    }

    async fn search_files(
        &self,
        request: Request<SearchFilesRequest>,
    ) -> Result<Response<SearchFilesResponse>, Status> {
        let req = request.into_inner();
        let workspace_dir = &self.state.config.workspace_dir;
        let full_pattern = format!("{workspace_dir}/{}", req.query);

        let files: Vec<String> = glob::glob(&full_pattern)
            .map(|paths| {
                paths
                    .filter_map(Result::ok)
                    .take(req.limit.max(1) as usize)
                    .filter_map(|p| {
                        p.strip_prefix(workspace_dir)
                            .ok()
                            .map(|rel| rel.to_string_lossy().to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(Response::new(SearchFilesResponse { files }))
    }

    async fn get_server_status(
        &self,
        request: Request<GetServerStatusRequest>,
    ) -> Result<Response<GetServerStatusResponse>, Status> {
        let name = request.into_inner().name;

        let status = self
            .state
            .servers
            .get_status(&name)
            .await
            .ok_or_else(|| Status::not_found(format!("Server not found: {name}")))?;

        let status_json = serde_json::to_string(&status)
            .map_err(|e| Status::internal(format!("Serialization error: {e}")))?;

        Ok(Response::new(GetServerStatusResponse { status_json }))
    }

    type StreamServerLogsStream =
        tokio_stream::wrappers::ReceiverStream<Result<LogLine, Status>>;

    async fn stream_server_logs(
        &self,
        request: Request<GetServerLogsRequest>,
    ) -> Result<Response<Self::StreamServerLogsStream>, Status> {
        let req = request.into_inner();
        let max_lines = req.lines.unwrap_or(500) as usize;

        let logs = self
            .state
            .servers
            .get_logs(&req.name, max_lines)
            .await
            .ok_or_else(|| Status::not_found(format!("Server not found: {}", req.name)))?;

        let (tx, rx) = tokio::sync::mpsc::channel(128);

        // Send each log line as a streaming response
        tokio::spawn(async move {
            for line in logs.lines() {
                if tx
                    .send(Ok(LogLine {
                        line: line.to_string(),
                    }))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });

        Ok(Response::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
    }
}
