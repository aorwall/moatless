use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use codex_app_server::in_process::InProcessServerEvent;
use codex_app_server_client::InProcessAppServerClient;
use codex_app_server_client::InProcessAppServerRequestHandle;
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
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;
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
    request_counter: AtomicI64,
    turns: TurnAdmission,
    orchestrator_handle: Option<JoinHandle<()>>,
}

impl QueryHandle {
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
                    responsesapi_client_metadata: None,
                    additional_context: None,
                    client_user_message_id: None,
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
        self.turns
            .submit(async {
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
            })
            .await
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
        let id = self.request_counter.fetch_add(1, Ordering::Relaxed);
        RequestId::Integer(id)
    }
}

struct TurnAdmission {
    maximum: Option<u32>,
    successful: Mutex<u64>,
}

impl TurnAdmission {
    async fn submit<T>(&self, request: impl Future<Output = Result<T>>) -> Result<T> {
        let mut successful = self.successful.lock().await;
        if self
            .maximum
            .is_some_and(|maximum| *successful >= u64::from(maximum))
        {
            return Err(SdkError::Request("maximum turn count reached".into()));
        }
        let response = request.await?;
        *successful = successful.saturating_add(1);
        Ok(response)
    }
}

/// Run a one-shot query: send a single prompt and stream all messages
/// until the turn completes.
///
/// This is the Codex equivalent of the Claude SDK's `query()`.
pub async fn query(prompt: &str, options: CodexOptions) -> Result<QueryHandle> {
    let mut handle = query_streaming(options).await?;
    if let Err(error) = handle.send(prompt).await {
        handle.shutdown().await;
        return Err(error);
    }
    Ok(handle)
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
        state.worker,
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
        request_counter: AtomicI64::new(200),
        turns: TurnAdmission {
            maximum: options.max_turns,
            successful: Mutex::new(0),
        },
        orchestrator_handle: Some(orchestrator_handle),
    })
}

/// Owns event delivery, approvals and runtime shutdown.
fn spawn_orchestrator(
    mut client: InProcessAppServerClient,
    worker: runtime::RuntimeWorker,
    approval_handler: Arc<dyn ApprovalHandler>,
    hook_observer: Option<Arc<dyn HookObserver>>,
    cancel_token: CancellationToken,
    msg_tx: mpsc::Sender<Result<CodexMessage>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = cancel_token.cancelled() => break,
                _ = msg_tx.closed() => break,
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
                            if !handoff(&cancel_token, &msg_tx, async { msg_tx.send(Ok(msg)).await.is_ok() }).await {
                                break;
                            }
                        }
                        InProcessServerEvent::ServerRequest(request) => {
                            if !handoff(&cancel_token, &msg_tx, async {
                                handle_server_request(&client, request, &approval_handler, &cancel_token).await;
                                true
                            }).await { break; }
                        }
                        InProcessServerEvent::Lagged { skipped } => {
                            warn!("codex-sdk: dropped {skipped} events due to backpressure");
                        }
                    }
                }
            }
        }
        let _ = client.shutdown().await;
        worker.shutdown().await;
    })
}

async fn handoff<T>(
    cancel: &CancellationToken,
    messages: &mpsc::Sender<T>,
    operation: impl Future<Output = bool>,
) -> bool {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => false,
        _ = messages.closed() => false,
        completed = operation => completed,
    }
}

async fn handle_server_request(
    client: &InProcessAppServerClient,
    request: ServerRequest,
    handler: &Arc<dyn ApprovalHandler>,
    cancel: &CancellationToken,
) {
    match request {
        ServerRequest::CommandExecutionRequestApproval { request_id, params } => {
            let decision = handler.on_command_approval(&params).await;
            if cancel.is_cancelled() {
                return;
            }
            let response = CommandExecutionRequestApprovalResponse { decision };
            if let Ok(result) = serde_json::to_value(response)
                && let Err(e) = client.resolve_server_request(request_id, result).await
            {
                warn!("failed to resolve command approval: {e}");
            }
        }
        ServerRequest::FileChangeRequestApproval { request_id, params } => {
            let decision = handler.on_file_change_approval(&params).await;
            if cancel.is_cancelled() {
                return;
            }
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
                    if cancel.is_cancelled() {
                        return;
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::{pending, poll_fn};
    use std::task::Poll;

    #[tokio::test]
    async fn turn_limit_counts_successes_and_serializes_concurrent_admission() {
        let turns = TurnAdmission {
            maximum: Some(1),
            successful: Mutex::new(0),
        };
        assert!(
            turns
                .submit(async { Err::<(), _>(SdkError::Request("rejected".into())) })
                .await
                .is_err()
        );
        let (first, second) = tokio::join!(
            turns.submit(async { Ok(()) }),
            turns.submit(async { Ok(()) })
        );
        assert!(first.is_ok());
        assert!(matches!(second, Err(SdkError::Request(_))));
        assert_eq!(*turns.successful.lock().await, 1);
    }

    #[tokio::test]
    async fn concurrent_request_waits_for_failed_reservation() {
        let turns = TurnAdmission {
            maximum: Some(1),
            successful: Mutex::new(0),
        };
        let (release, released) = tokio::sync::oneshot::channel();
        let first = turns.submit(async {
            released.await.unwrap();
            Err::<(), _>(SdkError::Request("rejected".into()))
        });
        let submitted = std::sync::atomic::AtomicBool::new(false);
        let second = turns.submit(async {
            submitted.store(true, Ordering::SeqCst);
            Ok(())
        });
        tokio::pin!(first, second);
        poll_fn(|cx| {
            assert!(first.as_mut().poll(cx).is_pending());
            assert!(second.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        assert!(!submitted.load(Ordering::SeqCst));
        release.send(()).unwrap();
        assert!(first.await.is_err());
        assert!(second.await.is_ok());
        assert_eq!(*turns.successful.lock().await, 1);
    }

    #[tokio::test]
    async fn zero_never_submits_and_none_is_unlimited() {
        let zero = TurnAdmission {
            maximum: Some(0),
            successful: Mutex::new(0),
        };
        assert!(
            zero.submit(poll_fn(|_| -> Poll<Result<()>> {
                panic!("zero must not submit")
            }))
            .await
            .is_err()
        );
        let unlimited = TurnAdmission {
            maximum: None,
            successful: Mutex::new(u64::from(u32::MAX)),
        };
        assert!(unlimited.submit(async { Ok(()) }).await.is_ok());
    }

    #[tokio::test]
    async fn full_queue_cancellation_unblocks_without_draining() {
        let cancel = CancellationToken::new();
        let (sender, mut receiver) = mpsc::channel(1);
        sender.send(1).await.unwrap();
        let delivery = handoff(&cancel, &sender, async { sender.send(2).await.is_ok() });
        tokio::pin!(delivery);
        poll_fn(|cx| {
            assert!(delivery.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        cancel.cancel();
        assert!(!delivery.await);
        assert_eq!(receiver.try_recv().unwrap(), 1);
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn pending_approval_is_dropped_without_acceptance() {
        let cancel = CancellationToken::new();
        let (sender, _receiver) = mpsc::channel::<()>(1);
        let accepted = std::sync::atomic::AtomicBool::new(false);
        let approval = handoff(&cancel, &sender, async {
            pending::<()>().await;
            accepted.store(true, Ordering::SeqCst);
            true
        });
        tokio::pin!(approval);
        poll_fn(|cx| {
            assert!(approval.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        cancel.cancel();
        assert!(!approval.await);
        assert!(!accepted.load(Ordering::SeqCst));
    }
}
