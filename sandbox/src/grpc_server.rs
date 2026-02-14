//! gRPC server implementing `SandboxService`.
//!
//! Receives commands from the backend (execute agent, stop agent, notify
//! messages, status checks, file search). Server management is handled
//! by K8s (separate containers), not by the sandbox.

use tonic::{Request, Response, Status};
use tracing::info;

use sandbox_proto::sandbox_service_server::SandboxService;
use sandbox_proto::{
    ExecuteAgentRequest, ExecuteAgentResponse, NotifyMessagesRequest, NotifyMessagesResponse,
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

    async fn search_files(
        &self,
        request: Request<SearchFilesRequest>,
    ) -> Result<Response<SearchFilesResponse>, Status> {
        let req = request.into_inner();
        let workspace_dir = &self.state.config.workspace_dir;

        // Reject path traversal attempts
        if req.query.contains("..") {
            return Err(Status::invalid_argument("Query must not contain '..'"));
        }

        let full_pattern = format!("{workspace_dir}/{}", req.query);
        let workspace_path = std::path::Path::new(workspace_dir);

        let files: Vec<String> = glob::glob(&full_pattern)
            .map(|paths| {
                paths
                    .filter_map(Result::ok)
                    .filter(|p| {
                        // Ensure resolved path is within workspace directory
                        p.canonicalize()
                            .ok()
                            .and_then(|canonical| workspace_path.canonicalize().ok().map(|ws| canonical.starts_with(ws)))
                            .unwrap_or(false)
                    })
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
}
