//! Interactive permission handler for AskUserQuestion and ExitPlanMode tools.
//!
//! When the agent encounters one of these tools, the handler creates a oneshot
//! channel, stores it as a pending prompt in `AgentState`, and blocks until
//! a response is received (or the agent is cancelled).
//!
//! In plan mode, write tools (Write, Edit, Bash, etc.) are blocked until
//! ExitPlanMode has been approved.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::agent_state::AgentState;
use super::prompt::{PendingPrompt, PromptResponse, PromptToolName};

/// Tools that require plan approval before use.
const WRITE_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit", "Bash"];

/// Permission handler that intercepts AskUserQuestion and ExitPlanMode tools.
///
/// When `plan_mode_active` is true, write tools are denied until the user
/// approves the plan via ExitPlanMode.
pub struct InteractivePermissionHandler {
    pub agent_state: Arc<AgentState>,
    /// When true, write tools are blocked pending ExitPlanMode approval.
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

    /// Create a handler without plan mode (all non-interactive tools auto-allowed).
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
        context: claude_agent_sdk::types::permissions::PermissionContext,
    ) -> claude_agent_sdk::PermissionResult {
        // Check if this is an interactive prompt tool
        if let Some(prompt_tool) = PromptToolName::from_tool_name(tool_name) {
            let input_value = serde_json::Value::Object(input.clone());

            // Create oneshot channel for the response
            let (tx, rx) = tokio::sync::oneshot::channel();

            let prompt = PendingPrompt {
                tool_name: prompt_tool,
                input: input_value,
                sender: tx,
            };

            // Store the pending prompt
            self.agent_state.set_pending_prompt(prompt).await;

            // Wait for the response (blocks until user responds or agent is cancelled)
            match rx.await {
                Ok(response) => {
                    // Convert prompt response to permission result
                    let result_json = match &response {
                        PromptResponse::AskUserAnswer { answers } => {
                            serde_json::json!({ "answers": answers })
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
            let _ = context; // suppress unused warning
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

    #[tokio::test]
    async fn plan_mode_denies_write_tool() {
        let state = Arc::new(AgentState::new());
        let handler = InteractivePermissionHandler::with_plan_mode(state, true);
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
        let handler = InteractivePermissionHandler::with_plan_mode(state, true);
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
        let handler = InteractivePermissionHandler::with_plan_mode(state, true);
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
        let handler = InteractivePermissionHandler::with_plan_mode(state, true);
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
        let handler = InteractivePermissionHandler::with_plan_mode(state, false);
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
        let handler = InteractivePermissionHandler::new(state);
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
        let handler = InteractivePermissionHandler::new(Arc::clone(&state));

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
            answers: serde_json::json!({ "text": "blue" }),
        });

        let result = handle.await.unwrap();
        assert!(
            matches!(result, claude_agent_sdk::PermissionResult::Allow { .. }),
            "AskUserQuestion should be allowed after response"
        );
    }

    #[tokio::test]
    async fn exit_plan_mode_deactivates_plan_mode() {
        let state = Arc::new(AgentState::new());
        let handler = InteractivePermissionHandler::with_plan_mode(Arc::clone(&state), true);
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
        let handler = InteractivePermissionHandler::new(Arc::clone(&state));

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
}
