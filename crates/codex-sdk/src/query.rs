use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use codex_app_server::in_process::InProcessServerEvent;
use codex_app_server_client::InProcessAppServerClient;
use codex_app_server_client::InProcessAppServerRequestHandle;
use tokio::task::JoinHandle;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::CommandExecutionRequestApprovalResponse;
use codex_app_server_protocol::FileChangeRequestApprovalResponse;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ServerRequest;
use codex_app_server_protocol::TurnStartParams;
use codex_app_server_protocol::TurnStartResponse;
use codex_app_server_protocol::UserInput;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::approvals::{ApprovalHandler, AutoApproveHandler};
use crate::error::{Result, SdkError};
use crate::hooks::HookObserver;
use crate::messages::CodexMessage;
use crate::options::CodexOptions;
use crate::runtime;

/// Handle to a running Codex query.
///
/// Provides the message stream and control methods (interrupt, steer).
/// This is the Codex equivalent of the Claude SDK's `QueryHandle`.
pub struct QueryHandle {
    /// Stream of messages from the agent. Receives `Ok(CodexMessage)` for
    /// each event, or `Err(SdkError)` on failure.
    pub messages: mpsc::Receiver<Result<CodexMessage>>,

    thread_id: String,
    request_handle: InProcessAppServerRequestHandle,
    cancellation_token: CancellationToken,
    turn_counter: AtomicI64,
    orchestrator_handle: Option<JoinHandle<()>>,
}

impl QueryHandle {
    /// Returns the thread ID for this session.
    pub fn thread_id(&self) -> &str {
        &self.thread_id
    }

    /// Interrupt the currently running turn.
    pub async fn interrupt(&self) -> Result<()> {
        let id = self.next_request_id();
        // Use empty turn_id — the server will interrupt whatever is active.
        let _ = self
            .request_handle
            .request(ClientRequest::TurnInterrupt {
                request_id: id,
                params: codex_app_server_protocol::TurnInterruptParams {
                    thread_id: self.thread_id.clone(),
                    turn_id: String::new(),
                },
            })
            .await
            .map_err(|e| SdkError::Request(format!("interrupt failed: {e}")))?;
        Ok(())
    }

    /// Inject additional context into the currently running turn (Codex-specific).
    pub async fn steer(&self, input: &str) -> Result<()> {
        let id = self.next_request_id();
        let _ = self
            .request_handle
            .request(ClientRequest::TurnSteer {
                request_id: id,
                params: codex_app_server_protocol::TurnSteerParams {
                    thread_id: self.thread_id.clone(),
                    input: vec![UserInput::Text {
                        text: input.to_string(),
                        text_elements: Vec::new(),
                    }],
                    expected_turn_id: String::new(),
                },
            })
            .await
            .map_err(|e| SdkError::Request(format!("steer failed: {e}")))?;
        Ok(())
    }

    /// Start a new turn with the given prompt text.
    /// Used in multi-turn mode (`query_streaming`).
    pub async fn send(&self, prompt: &str) -> Result<()> {
        let id = self.next_request_id();
        let _response: TurnStartResponse = self
            .request_handle
            .request_typed(ClientRequest::TurnStart {
                request_id: id,
                params: TurnStartParams {
                    thread_id: self.thread_id.clone(),
                    input: vec![UserInput::Text {
                        text: prompt.to_string(),
                        text_elements: Vec::new(),
                    }],
                    ..Default::default()
                },
            })
            .await
            .map_err(|e| SdkError::Request(format!("turn/start failed: {e}")))?;
        Ok(())
    }

    /// Signal cancellation and stop the orchestrator.
    pub fn cancel(&self) {
        self.cancellation_token.cancel();
    }

    /// Cancel and wait for the orchestrator and runtime to fully shut down.
    ///
    /// This ensures all resources (including the in-process app-server) are
    /// released before returning. Use this instead of `cancel()` when you need
    /// to guarantee cleanup (e.g. before starting a new session in the same
    /// process).
    pub async fn shutdown(&mut self) {
        self.cancellation_token.cancel();
        if let Some(handle) = self.orchestrator_handle.take() {
            let _ = handle.await;
        }
    }

    fn next_request_id(&self) -> RequestId {
        let id = self.turn_counter.fetch_add(1, Ordering::Relaxed);
        RequestId::Integer(id)
    }
}

/// Run a one-shot query: send a single prompt and stream all messages
/// until the turn completes.
///
/// This is the Codex equivalent of the Claude SDK's `query()`.
pub async fn query(prompt: &str, options: CodexOptions) -> Result<QueryHandle> {
    let state = runtime::start_runtime(&options).await?;
    let cancel_token = options.cancellation_token.clone().unwrap_or_default();
    let approval_handler: Arc<dyn ApprovalHandler> = options
        .approval_handler
        .clone()
        .unwrap_or_else(|| Arc::new(AutoApproveHandler));
    let hook_observer = options.hook_observer.clone();

    let request_handle = state.client.request_handle();

    // Start the initial turn.
    let _turn_response: TurnStartResponse = state
        .client
        .request_typed(ClientRequest::TurnStart {
            request_id: RequestId::Integer(100),
            params: TurnStartParams {
                thread_id: state.thread_id.clone(),
                input: vec![UserInput::Text {
                    text: prompt.to_string(),
                    text_elements: Vec::new(),
                }],
                ..Default::default()
            },
        })
        .await
        .map_err(|e| SdkError::Request(format!("turn/start failed: {e}")))?;

    let (msg_tx, msg_rx) = mpsc::channel(256);

    let orchestrator_handle = spawn_orchestrator(
        state.client,
        approval_handler,
        hook_observer,
        cancel_token.clone(),
        msg_tx,
    );

    Ok(QueryHandle {
        messages: msg_rx,
        thread_id: state.thread_id,
        request_handle,
        cancellation_token: cancel_token,
        turn_counter: AtomicI64::new(200),
        orchestrator_handle: Some(orchestrator_handle),
    })
}

/// Create a multi-turn streaming session. No initial prompt is sent;
/// use `QueryHandle::send()` to start turns.
///
/// This is the Codex equivalent of the Claude SDK's `query_streaming()`.
pub async fn query_streaming(options: CodexOptions) -> Result<QueryHandle> {
    let state = runtime::start_runtime(&options).await?;
    let cancel_token = options.cancellation_token.clone().unwrap_or_default();
    let approval_handler: Arc<dyn ApprovalHandler> = options
        .approval_handler
        .clone()
        .unwrap_or_else(|| Arc::new(AutoApproveHandler));
    let hook_observer = options.hook_observer.clone();

    let request_handle = state.client.request_handle();

    let (msg_tx, msg_rx) = mpsc::channel(256);

    let orchestrator_handle = spawn_orchestrator(
        state.client,
        approval_handler,
        hook_observer,
        cancel_token.clone(),
        msg_tx,
    );

    Ok(QueryHandle {
        messages: msg_rx,
        thread_id: state.thread_id,
        request_handle,
        cancellation_token: cancel_token,
        turn_counter: AtomicI64::new(200),
        orchestrator_handle: Some(orchestrator_handle),
    })
}

/// Spawns the background orchestrator task that:
/// 1. Consumes `InProcessServerEvent` from the app-server client
/// 2. Maps `ServerNotification` -> `CodexMessage` and forwards to the user channel
/// 3. Routes `ServerRequest` (approvals) to the `ApprovalHandler` and resolves them
/// 4. Dispatches hook notifications to the `HookObserver`
fn spawn_orchestrator(
    mut client: InProcessAppServerClient,
    approval_handler: Arc<dyn ApprovalHandler>,
    hook_observer: Option<Arc<dyn HookObserver>>,
    cancel_token: CancellationToken,
    msg_tx: mpsc::Sender<Result<CodexMessage>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel_token.cancelled() => {
                    let _ = client.shutdown().await;
                    break;
                }
                event = client.next_event() => {
                    let Some(event) = event else {
                        break;
                    };
                    match event {
                        InProcessServerEvent::ServerNotification(notification) => {
                            if let Some(ref observer) = hook_observer {
                                match &notification {
                                    ServerNotification::HookStarted(n) => {
                                        observer.on_hook_started(n);
                                    }
                                    ServerNotification::HookCompleted(n) => {
                                        observer.on_hook_completed(n);
                                    }
                                    _ => {}
                                }
                            }
                            let msg = CodexMessage::from(notification);
                            if msg_tx.send(Ok(msg)).await.is_err() {
                                let _ = client.shutdown().await;
                                break;
                            }
                        }
                        InProcessServerEvent::ServerRequest(request) => {
                            handle_server_request(&client, request, &approval_handler).await;
                        }
                        InProcessServerEvent::Lagged { skipped } => {
                            warn!("codex-sdk: dropped {skipped} events due to backpressure");
                        }
                    }
                }
            }
        }
    })
}

async fn handle_server_request(
    client: &InProcessAppServerClient,
    request: ServerRequest,
    handler: &Arc<dyn ApprovalHandler>,
) {
    match request {
        ServerRequest::CommandExecutionRequestApproval { request_id, params } => {
            let decision = handler.on_command_approval(&params).await;
            let response = CommandExecutionRequestApprovalResponse { decision };
            if let Ok(result) = serde_json::to_value(response)
                && let Err(e) = client.resolve_server_request(request_id, result).await
            {
                warn!("failed to resolve command approval: {e}");
            }
        }
        ServerRequest::FileChangeRequestApproval { request_id, params } => {
            let decision = handler.on_file_change_approval(&params).await;
            let response = FileChangeRequestApprovalResponse { decision };
            if let Ok(result) = serde_json::to_value(response)
                && let Err(e) = client.resolve_server_request(request_id, result).await
            {
                warn!("failed to resolve file change approval: {e}");
            }
        }
        ServerRequest::PermissionsRequestApproval { request_id, params } => {
            match handler.on_permissions_approval(&params).await {
                Some(response) => {
                    if let Ok(result) = serde_json::to_value(response)
                        && let Err(e) = client.resolve_server_request(request_id, result).await
                    {
                        warn!("failed to resolve permissions approval: {e}");
                    }
                }
                None => {
                    let _ = client
                        .reject_server_request(
                            request_id,
                            JSONRPCErrorError {
                                code: -32000,
                                message: "permissions request denied by SDK approval handler"
                                    .to_string(),
                                data: None,
                            },
                        )
                        .await;
                }
            }
        }
        other => {
            let _ = client
                .reject_server_request(
                    other.id().clone(),
                    JSONRPCErrorError {
                        code: -32601,
                        message: "unsupported server request in SDK".to_string(),
                        data: None,
                    },
                )
                .await;
        }
    }
}
