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
        } else if self.plan_mode_active.load(Ordering::SeqCst)
            && WRITE_TOOLS.contains(&tool_name)
        {
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
