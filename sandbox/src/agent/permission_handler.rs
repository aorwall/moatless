//! Interactive permission handler for AskUserQuestion and ExitPlanMode.
//!
//! When the agent encounters AskUserQuestion or ExitPlanMode, the handler
//! creates a oneshot channel, stores it as a pending prompt in `AgentState`,
//! and blocks until a response is received (or the agent is cancelled).
//!
//! In plan mode, write tools (Write, Edit, Bash, etc.) are blocked until
//! ExitPlanMode has been approved.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use serde_json::json;

use super::agent_state::AgentState;
use super::prompt::{PendingPrompt, PromptResponse, PromptToolName};
use crate::events::GrpcEventPublisher;

/// Tools that require plan approval before use.
const WRITE_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit", "Bash"];

/// Where the call that parked the run is announced, so a person can see the
/// question they are being asked.
///
/// Nothing else guarantees it reaches them in time. The block that holds the
/// call travels the harness stream on its own schedule — it may not have been
/// written when permission is asked for, and the CLI is waiting on this answer
/// before it writes anything more. Announcing from here puts the question in
/// front of a person the moment it is asked, whatever the stream does.
struct PromptSink {
    events: GrpcEventPublisher,
    task_id: String,
}

/// Permission handler that intercepts AskUserQuestion and ExitPlanMode.
///
/// When `plan_mode_active` is true, write tools are denied until the user
/// approves the plan via ExitPlanMode.
pub struct InteractivePermissionHandler {
    pub agent_state: Arc<AgentState>,
    /// When true, write tools are blocked pending ExitPlanMode approval.
    pub plan_mode_active: Arc<AtomicBool>,
    /// `None` in tests and wherever no publisher is wired; the prompt then
    /// parks exactly as before, unannounced.
    prompt_sink: Option<PromptSink>,
}

impl InteractivePermissionHandler {
    /// Create a handler with plan mode enforcement.
    pub fn with_plan_mode(agent_state: Arc<AgentState>, plan_mode: bool) -> Self {
        Self {
            agent_state,
            plan_mode_active: Arc::new(AtomicBool::new(plan_mode)),
            prompt_sink: None,
        }
    }

    /// Create a handler without plan mode (all non-interactive tools auto-allowed).
    pub fn new(agent_state: Arc<AgentState>) -> Self {
        Self::with_plan_mode(agent_state, false)
    }

    /// Announce every interactive call this handler parks on through `events`.
    #[must_use]
    pub fn announcing_prompts(mut self, events: GrpcEventPublisher, task_id: String) -> Self {
        self.prompt_sink = Some(PromptSink { events, task_id });
        self
    }

    /// Publish the parked call as the `tool_use` block it is.
    ///
    /// Shaped exactly like the per-block `agent.message` the coordinator
    /// publishes for an ordinary block, so the backend's existing mapping does
    /// the rest: the call row the answer settles, and the joined
    /// `user_input_requested` row the answer panel is derived from.
    ///
    /// The message uuid is derived from the tool-use id rather than allocated,
    /// so a redelivery of the announcement itself updates the row rather than
    /// adding a second. The *real* block carrying this call is a different
    /// message and cannot be deduplicated that way; the two settle it with a
    /// claim instead, and the one that gets there first publishes.
    ///
    /// Run-scoped through `current_run_id` so the call lands on the turn it
    /// belongs to, the same attribution an ack of the answer already uses.
    async fn announce(&self, tool_name: &str, tool_use_id: &str, input: &serde_json::Value) {
        let Some(sink) = &self.prompt_sink else {
            return;
        };
        if tool_use_id.is_empty() {
            tracing::warn!(
                tool = %tool_name,
                "Interactive call carries no tool-use id; parking it unannounced"
            );
            return;
        }
        // The block holding this call reaches the coordinator by its own route.
        // Claiming the id here is what keeps the two from writing the same call
        // twice; losing the claim means the block got there first, and there is
        // nothing left to announce.
        if !self.agent_state.claim_prompt_publication(tool_use_id).await {
            return;
        }

        let events = match self.agent_state.current_run_id().await {
            Some(run_id) => sink.events.for_run(&run_id),
            None => sink.events.clone(),
        };
        let published = events
            .publish_agent_event(
                &sink.task_id,
                moatless_types::agent::AgentSubType::Message,
                prompt_message_payload(
                    tool_name,
                    tool_use_id,
                    input,
                    &self.agent_state.last_session_id().await.unwrap_or_default(),
                ),
            )
            .await;
        // The claim was taken on the strength of a publication that then did not
        // happen. Hand it back so the block carrying this call still publishes
        // when the harness delivers it.
        if !published {
            self.agent_state
                .release_prompt_publication(tool_use_id)
                .await;
            tracing::warn!(
                tool = %tool_name,
                tool_use_id,
                "Interactive call announcement was not enqueued; leaving the block to publish it"
            );
        }
    }
}

/// The `agent.message` payload announcing one parked interactive call.
///
/// Byte-for-byte the shape `coordinator::message_payload` publishes for an
/// ordinary per-block source event, because it is read by the same handler:
/// one `blockIndex`, one `content` block, the message's own uuid on the outside
/// and the inside.
fn prompt_message_payload(
    tool_name: &str,
    tool_use_id: &str,
    input: &serde_json::Value,
    session_id: &str,
) -> serde_json::Value {
    let uuid = format!("prompt-{tool_use_id}");
    json!({
        "uuid": uuid,
        "blockIndex": 0,
        "session_id": session_id,
        "message": {
            "role": "assistant",
            "id": uuid,
            "content": [{
                "type": "tool_use",
                "id": tool_use_id,
                "name": tool_name,
                "input": input,
            }],
        },
    })
}

#[async_trait::async_trait]
impl claude_agent_sdk::PermissionHandler for InteractivePermissionHandler {
    async fn can_use_tool(
        &self,
        tool_name: &str,
        input: &serde_json::Map<String, serde_json::Value>,
        context: claude_agent_sdk::types::permissions::PermissionContext,
    ) -> claude_agent_sdk::PermissionResult {
        // Check if this is an interactive prompt tool
        if let Some(prompt_tool) = PromptToolName::from_tool_name(tool_name) {
            let input_value = serde_json::Value::Object(input.clone());

            // Create oneshot channel for the response
            let (tx, rx) = tokio::sync::oneshot::channel();

            let prompt = PendingPrompt {
                tool_name: prompt_tool,
                tool_use_id: context.tool_use_id.clone(),
                input: input_value.clone(),
                sender: tx,
            };

            // Store the pending prompt. A question already standing keeps its
            // place: answering it is what lets the run continue, and there is
            // one panel to answer it in.
            if !self.agent_state.set_pending_prompt(prompt).await {
                return claude_agent_sdk::PermissionResult::Deny {
                    message: "Another question is already waiting to be answered. \
                              Ask this one again once that answer arrives."
                        .to_string(),
                    interrupt: None,
                };
            }

            // Say what is being asked before waiting to be answered: the wait
            // below ends only when a person answers, and the panel they answer
            // in is derived from this.
            self.announce(tool_name, &context.tool_use_id, &input_value)
                .await;

            // Nothing is running while this waits, and the idle reaper asks the
            // agent before pulling the plug: reported as running, a parked run
            // holds its pod past every TTL. The status is restored on the way
            // out so the rest of the turn reports itself normally.
            self.agent_state.begin_waiting().await;

            // Wait for the response (blocks until user responds or agent is
            // cancelled). No timeout: a person who answers while the sandbox is
            // alive continues the same turn, and a sandbox reclaimed underneath
            // an unanswered question leaves the question standing — it is stored,
            // and answering it later starts a fresh turn carrying it.
            let response = rx.await;
            self.agent_state.end_waiting().await;
            match response {
                Ok(response) => {
                    // Convert prompt response to permission result
                    let result_json = match &response {
                        PromptResponse::AskUserAnswer { answers } => {
                            serde_json::json!({ "answers": answers })
                        }
                        PromptResponse::AskUserResponse { response } => {
                            serde_json::json!({ "response": response })
                        }
                        PromptResponse::PlanApproved => {
                            serde_json::json!({ "approved": true })
                        }
                        PromptResponse::PlanRejected { feedback } => {
                            serde_json::json!({ "approved": false, "feedback": feedback })
                        }
                    };

                    // If plan was approved, deactivate plan mode restrictions
                    if matches!(response, PromptResponse::PlanApproved) {
                        self.plan_mode_active.store(false, Ordering::SeqCst);
                    }

                    let mut updated_input = input.clone();
                    // Merge the response into the input
                    if let serde_json::Value::Object(map) = result_json {
                        for (k, v) in map {
                            updated_input.insert(k, v);
                        }
                    }

                    claude_agent_sdk::PermissionResult::Allow {
                        updated_input: Some(updated_input),
                        updated_permissions: None,
                    }
                }
                Err(_) => {
                    // Channel closed — agent was cancelled
                    claude_agent_sdk::PermissionResult::Deny {
                        message: "Agent was cancelled".to_string(),
                        interrupt: Some(true),
                    }
                }
            }
        } else if self.plan_mode_active.load(Ordering::SeqCst) && WRITE_TOOLS.contains(&tool_name) {
            // In plan mode, deny write tools until ExitPlanMode is approved
            claude_agent_sdk::PermissionResult::Deny {
                message: "Plan mode: use ExitPlanMode to get plan approval before making changes."
                    .to_string(),
                interrupt: None,
            }
        } else {
            // Not an interactive tool, not blocked by plan mode — auto-allow
            claude_agent_sdk::PermissionResult::Allow {
                updated_input: None,
                updated_permissions: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claude_agent_sdk::types::permissions::PermissionContext;
    use claude_agent_sdk::PermissionHandler;

    fn test_context() -> PermissionContext {
        PermissionContext {
            tool_use_id: "tu-1".to_string(),
            agent_id: None,
            suggestions: None,
            blocked_path: None,
            decision_reason: None,
        }
    }

    fn empty_input() -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::new()
    }

    fn test_handler(state: Arc<AgentState>) -> InteractivePermissionHandler {
        InteractivePermissionHandler::new(state)
    }

    fn test_handler_with_plan_mode(
        state: Arc<AgentState>,
        plan_mode: bool,
    ) -> InteractivePermissionHandler {
        InteractivePermissionHandler::with_plan_mode(state, plan_mode)
    }

    #[tokio::test]
    async fn plan_mode_denies_write_tool() {
        let state = Arc::new(AgentState::new());
        let handler = test_handler_with_plan_mode(state, true);
        let result = handler
            .can_use_tool("Write", &empty_input(), test_context())
            .await;
        assert!(
            matches!(result, claude_agent_sdk::PermissionResult::Deny { .. }),
            "Write should be denied in plan mode"
        );
    }

    #[tokio::test]
    async fn plan_mode_denies_bash_tool() {
        let state = Arc::new(AgentState::new());
        let handler = test_handler_with_plan_mode(state, true);
        let result = handler
            .can_use_tool("Bash", &empty_input(), test_context())
            .await;
        assert!(matches!(
            result,
            claude_agent_sdk::PermissionResult::Deny { .. }
        ));
    }

    #[tokio::test]
    async fn plan_mode_denies_edit_tool() {
        let state = Arc::new(AgentState::new());
        let handler = test_handler_with_plan_mode(state, true);
        let result = handler
            .can_use_tool("Edit", &empty_input(), test_context())
            .await;
        assert!(matches!(
            result,
            claude_agent_sdk::PermissionResult::Deny { .. }
        ));
    }

    #[tokio::test]
    async fn plan_mode_allows_read_tool() {
        let state = Arc::new(AgentState::new());
        let handler = test_handler_with_plan_mode(state, true);
        let result = handler
            .can_use_tool("Read", &empty_input(), test_context())
            .await;
        assert!(
            matches!(result, claude_agent_sdk::PermissionResult::Allow { .. }),
            "Read should be allowed even in plan mode"
        );
    }

    #[tokio::test]
    async fn no_plan_mode_allows_write_tool() {
        let state = Arc::new(AgentState::new());
        let handler = test_handler_with_plan_mode(state, false);
        let result = handler
            .can_use_tool("Write", &empty_input(), test_context())
            .await;
        assert!(matches!(
            result,
            claude_agent_sdk::PermissionResult::Allow { .. }
        ));
    }

    #[tokio::test]
    async fn non_interactive_tool_auto_allowed() {
        let state = Arc::new(AgentState::new());
        let handler = test_handler(state);
        let result = handler
            .can_use_tool("Grep", &empty_input(), test_context())
            .await;
        assert!(matches!(
            result,
            claude_agent_sdk::PermissionResult::Allow {
                updated_input: None,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn ask_user_question_creates_pending_prompt() {
        let state = Arc::new(AgentState::new());
        let handler = test_handler(Arc::clone(&state));

        // Spawn handler in background — it will block on the oneshot
        let handle = tokio::spawn(async move {
            handler
                .can_use_tool("AskUserQuestion", &empty_input(), test_context())
                .await
        });

        // Wait for the prompt to appear
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(state.has_pending_prompt().await, "Prompt should be pending");

        // Resolve the prompt
        let prompt = state.take_pending_prompt().await.unwrap();
        let _ = prompt.sender.send(PromptResponse::AskUserAnswer {
            answers: serde_json::json!({ "Which colour?": "blue" }),
        });

        let result = handle.await.unwrap();
        assert!(
            matches!(result, claude_agent_sdk::PermissionResult::Allow { .. }),
            "AskUserQuestion should be allowed after response"
        );
    }

    /// Where each shape of answer lands in the input the tool is finally run
    /// with. The questions it was asked with survive both, because the merge
    /// starts from that input: the CLI needs them to render what was answered.
    #[tokio::test]
    async fn an_answer_lands_in_the_field_the_cli_reads_it_from() {
        let asked = || {
            let mut input = serde_json::Map::new();
            input.insert(
                "questions".into(),
                serde_json::json!([{ "question": "Which store?" }]),
            );
            input
        };

        for (sent, field, expected) in [
            (
                PromptResponse::AskUserAnswer {
                    answers: serde_json::json!({ "Which store?": "Postgres" }),
                },
                "answers",
                serde_json::json!({ "Which store?": "Postgres" }),
            ),
            (
                PromptResponse::AskUserResponse {
                    response: "use Postgres".to_string(),
                },
                "response",
                serde_json::json!("use Postgres"),
            ),
        ] {
            let state = Arc::new(AgentState::new());
            let handler = test_handler(Arc::clone(&state));
            let handle = tokio::spawn(async move {
                handler
                    .can_use_tool("AskUserQuestion", &asked(), test_context())
                    .await
            });

            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            let prompt = state.take_pending_prompt().await.unwrap();
            let _ = prompt.sender.send(sent);

            let claude_agent_sdk::PermissionResult::Allow {
                updated_input: Some(input),
                ..
            } = handle.await.unwrap()
            else {
                panic!("an answered question runs the tool");
            };
            assert_eq!(input[field], expected);
            assert_eq!(input["questions"][0]["question"], "Which store?");
        }
    }

    #[tokio::test]
    async fn exit_plan_mode_deactivates_plan_mode() {
        let state = Arc::new(AgentState::new());
        let handler = test_handler_with_plan_mode(Arc::clone(&state), true);
        let plan_mode = Arc::clone(&handler.plan_mode_active);

        assert!(plan_mode.load(std::sync::atomic::Ordering::SeqCst));

        let handle = tokio::spawn(async move {
            handler
                .can_use_tool("ExitPlanMode", &empty_input(), test_context())
                .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let prompt = state.take_pending_prompt().await.unwrap();
        let _ = prompt.sender.send(PromptResponse::PlanApproved);

        let result = handle.await.unwrap();
        assert!(matches!(
            result,
            claude_agent_sdk::PermissionResult::Allow { .. }
        ));
        assert!(
            !plan_mode.load(std::sync::atomic::Ordering::SeqCst),
            "Plan mode should be deactivated after approval"
        );
    }

    #[tokio::test]
    async fn cancelled_prompt_returns_deny() {
        let state = Arc::new(AgentState::new());
        let handler = test_handler(Arc::clone(&state));

        let handle = tokio::spawn(async move {
            handler
                .can_use_tool("AskUserQuestion", &empty_input(), test_context())
                .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Drop the prompt sender to simulate cancellation
        let prompt = state.take_pending_prompt().await.unwrap();
        drop(prompt.sender);

        let result = handle.await.unwrap();
        assert!(
            matches!(
                result,
                claude_agent_sdk::PermissionResult::Deny {
                    interrupt: Some(true),
                    ..
                }
            ),
            "Cancelled prompt should return Deny with interrupt"
        );
    }

    /// The announcement carries the call itself, in the shape the backend's
    /// per-block `agent.message` handler already reads — which is what makes
    /// the existing mapping derive the answer panel from it without a second
    /// code path. The uuid is the tool-use id's, so the row a redelivery (or
    /// the real block, arriving late once the wait ends) lands on is the same
    /// one.
    #[test]
    fn a_parked_call_is_announced_as_the_block_it_is() {
        let input = serde_json::json!({ "plan": "do the thing" });
        let payload = prompt_message_payload("ExitPlanMode", "toolu_plan", &input, "sess-1");

        assert_eq!(payload["uuid"], "prompt-toolu_plan");
        assert_eq!(payload["blockIndex"], 0);
        assert_eq!(payload["session_id"], "sess-1");
        assert_eq!(payload["message"]["id"], "prompt-toolu_plan");
        assert_eq!(payload["message"]["role"], "assistant");

        let block = &payload["message"]["content"][0];
        assert_eq!(block["type"], "tool_use");
        assert_eq!(block["id"], "toolu_plan");
        assert_eq!(block["name"], "ExitPlanMode");
        assert_eq!(
            block["input"], input,
            "the plan the panel renders is the call's own input"
        );

        assert_eq!(
            prompt_message_payload("AskUserQuestion", "toolu_ask", &input, "sess-1")["uuid"],
            "prompt-toolu_ask",
            "each call is announced under its own id"
        );
    }

    /// Without a publisher the prompt still parks — the announcement is an
    /// addition to that path, never a precondition for it.
    #[tokio::test]
    async fn a_handler_with_no_sink_still_parks_the_prompt() {
        let state = Arc::new(AgentState::new());
        let handler = test_handler(Arc::clone(&state));
        assert!(handler.prompt_sink.is_none());

        let handle = tokio::spawn(async move {
            handler
                .can_use_tool("ExitPlanMode", &empty_input(), test_context())
                .await
        });

        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(state.has_pending_prompt().await, "the prompt still parks");

        let prompt = state.take_pending_prompt().await.unwrap();
        let _ = prompt.sender.send(PromptResponse::PlanApproved);
        handle.await.unwrap();
    }

    /// Two interactive calls in one assistant message reach the handler
    /// together, because permission decisions are made off the SDK's read loop.
    /// The one already waiting on a person keeps its place — replacing it would
    /// drop the sender its own run is waiting on, which reads as a cancellation.
    #[tokio::test]
    async fn a_second_question_is_refused_while_the_first_is_unanswered() {
        let state = Arc::new(AgentState::new());
        let first = test_handler(Arc::clone(&state));
        let parked = tokio::spawn(async move {
            first
                .can_use_tool("AskUserQuestion", &empty_input(), test_context())
                .await
        });
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(state.has_pending_prompt().await);

        let second = test_handler(Arc::clone(&state));
        let result = second
            .can_use_tool("ExitPlanMode", &empty_input(), test_context())
            .await;
        assert!(
            matches!(
                result,
                claude_agent_sdk::PermissionResult::Deny {
                    interrupt: None,
                    ..
                }
            ),
            "the second call is refused, and the turn is not interrupted"
        );

        // The first question is untouched, and answering it still works.
        let prompt = state.take_pending_prompt().await.expect("still parked");
        assert!(matches!(prompt.tool_name, PromptToolName::AskUserQuestion));
        let _ = prompt.sender.send(PromptResponse::AskUserAnswer {
            answers: serde_json::json!({ "Which colour?": "blue" }),
        });
        assert!(matches!(
            parked.await.unwrap(),
            claude_agent_sdk::PermissionResult::Allow { .. }
        ));
    }

    /// The claim says the call reached the backend by this route, and the block
    /// carrying it stays quiet on the strength of that. So an announcement that
    /// never went anywhere has to hand the claim back — otherwise the call is
    /// suppressed on both routes and nobody is ever asked the question.
    #[tokio::test]
    async fn a_refused_announcement_hands_the_claim_back() {
        let backend = Arc::new(crate::events::test_backend::CapturingBackend::default());
        let cancel = tokio_util::sync::CancellationToken::new();
        let (outbox, drainer) = crate::outbox::EventOutbox::spawn(
            backend as Arc<dyn crate::backend_client::BackendClient>,
            "task-1".to_string(),
            cancel.clone(),
        );
        // A stopped drainer refuses everything submitted after it: the outbox
        // will not take the announcement.
        cancel.cancel();
        drainer.shutdown(std::time::Duration::from_secs(5)).await;

        let state = Arc::new(AgentState::new());
        let handler = test_handler(Arc::clone(&state))
            .announcing_prompts(GrpcEventPublisher::new(outbox), "task-1".to_string());
        handler
            .announce("AskUserQuestion", "tu-1", &serde_json::json!({}))
            .await;

        assert!(
            state.claim_prompt_publication("tu-1").await,
            "the claim is free again, so the block still publishes the call"
        );
    }
}
