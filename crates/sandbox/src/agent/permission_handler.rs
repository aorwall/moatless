//! Interactive permission handler for `AskUserQuestion` and `ExitPlanMode`.
//!
//! When the agent encounters `AskUserQuestion` or `ExitPlanMode`, the handler
//! creates a oneshot channel, stores it as a pending prompt in `AgentState`,
//! and blocks until a response is received (or the agent is cancelled).
//!
//! In plan mode, write tools (Write, Edit, Bash, etc.) are blocked until
//! `ExitPlanMode` has been approved.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::agent_state::AgentState;
use super::prompt::{PendingPrompt, PromptResponse, PromptToolName};

/// Tools that require plan approval before use.
const WRITE_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit", "Bash"];

/// Permission handler that intercepts `AskUserQuestion` and `ExitPlanMode`.
pub struct InteractivePermissionHandler {
    pub agent_state: Arc<AgentState>,
    /// When true, write tools are blocked pending `ExitPlanMode` approval.
    pub plan_mode_active: Arc<AtomicBool>,
}

impl InteractivePermissionHandler {
    /// Create a handler with plan mode enforcement.
    pub fn with_plan_mode(agent_state: Arc<AgentState>, plan_mode: bool) -> Self {
        Self {
            agent_state,
            plan_mode_active: Arc::new(AtomicBool::new(plan_mode)),
        }
    }

    /// Create a handler without plan mode (all non-interactive tools
    /// auto-allowed).
    pub fn new(agent_state: Arc<AgentState>) -> Self {
        Self::with_plan_mode(agent_state, false)
    }
}

#[async_trait::async_trait]
impl claude_agent_sdk::PermissionHandler for InteractivePermissionHandler {
    async fn can_use_tool(
        &self,
        tool_name: &str,
        input: &serde_json::Map<String, serde_json::Value>,
        context: claude_agent_sdk::api::permissions::PermissionContext,
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
    use claude_agent_sdk::PermissionHandler;
    use claude_agent_sdk::api::permissions::PermissionContext;

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

    #[tokio::test]
    async fn an_interactive_call_parks_the_prompt() {
        let state = Arc::new(AgentState::new());
        let handler = test_handler(Arc::clone(&state));

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

    /// Two interactive calls in one message reach the handler together;
    /// permission decisions run off the SDK's read loop.
    ///
    /// The one already waiting on a person keeps its place — replacing it drops
    /// the sender its own run awaits, reading as a cancellation.
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
}
