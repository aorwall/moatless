//! Interactive permission handler for AskUserQuestion, ExitPlanMode, and
//! Monitor tools.
//!
//! When the agent encounters AskUserQuestion or ExitPlanMode, the handler
//! creates a oneshot channel, stores it as a pending prompt in `AgentState`,
//! and blocks until a response is received (or the agent is cancelled).
//!
//! When the agent encounters Monitor, the handler instead registers a cron
//! job against the shared `ScheduledJobRegistry` and rewrites the tool input
//! so the SDK's built-in implementation exits immediately. The cron's output
//! is delivered back to the agent later as a synthetic user message.
//!
//! In plan mode, write tools (Write, Edit, Bash, etc.) are blocked until
//! ExitPlanMode has been approved.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::agent_state::AgentState;
use super::monitor_job::{noop_monitor_input, MonitorJob, MonitorRequest};
use super::prompt::{PendingPrompt, PromptResponse, PromptToolName};
use super::scheduled_job::ScheduledJobRegistry;

/// Tools that require plan approval before use.
const WRITE_TOOLS: &[&str] = &["Write", "Edit", "MultiEdit", "NotebookEdit", "Bash"];

/// Built-in tool name we reroute into a sandbox cron.
const MONITOR_TOOL: &str = "Monitor";

/// Permission handler that intercepts AskUserQuestion, ExitPlanMode, and Monitor.
///
/// When `plan_mode_active` is true, write tools are denied until the user
/// approves the plan via ExitPlanMode.
pub struct InteractivePermissionHandler {
    pub agent_state: Arc<AgentState>,
    /// When true, write tools are blocked pending ExitPlanMode approval.
    pub plan_mode_active: Arc<AtomicBool>,
    /// Registry that owns deferred-work jobs (Monitor crons, ScheduleWakeup
    /// timers). Cancelled when the agent run ends.
    pub scheduled_jobs: Arc<ScheduledJobRegistry>,
    /// Cancellation token for the current agent run; passed into Monitor
    /// jobs so they stop when the run does.
    pub run_cancel: CancellationToken,
}

impl InteractivePermissionHandler {
    /// Create a handler with plan mode enforcement.
    pub fn with_plan_mode(
        agent_state: Arc<AgentState>,
        plan_mode: bool,
        scheduled_jobs: Arc<ScheduledJobRegistry>,
        run_cancel: CancellationToken,
    ) -> Self {
        Self {
            agent_state,
            plan_mode_active: Arc::new(AtomicBool::new(plan_mode)),
            scheduled_jobs,
            run_cancel,
        }
    }

    /// Create a handler without plan mode (all non-interactive tools auto-allowed).
    pub fn new(
        agent_state: Arc<AgentState>,
        scheduled_jobs: Arc<ScheduledJobRegistry>,
        run_cancel: CancellationToken,
    ) -> Self {
        Self::with_plan_mode(agent_state, false, scheduled_jobs, run_cancel)
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
        // Reroute Monitor calls into the sandbox-local cron registry. The
        // SDK still invokes the tool with the rewritten no-op input so the
        // model gets a normal tool_result; the actual recurring command runs
        // in a background tokio task that injects results later.
        if tool_name == MONITOR_TOOL {
            match MonitorRequest::from_tool_input(&context.tool_use_id, input) {
                Some(req) => {
                    let job = MonitorJob::from_request(req);
                    self.scheduled_jobs
                        .register(Box::new(job), self.run_cancel.clone())
                        .await;
                    return claude_agent_sdk::PermissionResult::Allow {
                        updated_input: Some(noop_monitor_input(input)),
                        updated_permissions: None,
                    };
                }
                None => {
                    // Malformed Monitor input — let the SDK report it.
                    return claude_agent_sdk::PermissionResult::Allow {
                        updated_input: None,
                        updated_permissions: None,
                    };
                }
            }
        }

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

    fn null_jobs_registry() -> Arc<ScheduledJobRegistry> {
        struct NullInjector;
        impl super::super::scheduled_job::JobInjector for NullInjector {
            fn inject(&self, _text: String) {}
        }
        Arc::new(ScheduledJobRegistry::new(Arc::new(NullInjector)))
    }

    fn test_handler(state: Arc<AgentState>) -> InteractivePermissionHandler {
        InteractivePermissionHandler::new(state, null_jobs_registry(), CancellationToken::new())
    }

    fn test_handler_with_plan_mode(
        state: Arc<AgentState>,
        plan_mode: bool,
    ) -> InteractivePermissionHandler {
        InteractivePermissionHandler::with_plan_mode(
            state,
            plan_mode,
            null_jobs_registry(),
            CancellationToken::new(),
        )
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
    async fn monitor_tool_registers_cron_and_rewrites_input() {
        use super::super::scheduled_job::JobInjector;

        #[derive(Default, Clone)]
        struct Capturing(Arc<std::sync::Mutex<Vec<String>>>);
        impl JobInjector for Capturing {
            fn inject(&self, text: String) {
                self.0.lock().expect("mutex poisoned").push(text);
            }
        }

        let injector = Capturing::default();
        let registry = Arc::new(ScheduledJobRegistry::new(Arc::new(injector.clone())));
        let handler = InteractivePermissionHandler::new(
            Arc::new(AgentState::new()),
            Arc::clone(&registry),
            CancellationToken::new(),
        );

        let mut input = serde_json::Map::new();
        input.insert(
            "command".to_string(),
            serde_json::Value::String("sleep 60".to_string()),
        );
        input.insert(
            "description".to_string(),
            serde_json::Value::String("waiting".to_string()),
        );
        input.insert(
            "timeout_ms".to_string(),
            serde_json::Value::Number(serde_json::Number::from(600_000u64)),
        );
        input.insert("persistent".to_string(), serde_json::Value::Bool(false));

        let result = handler.can_use_tool("Monitor", &input, test_context()).await;

        match result {
            claude_agent_sdk::PermissionResult::Allow {
                updated_input: Some(updated),
                ..
            } => {
                assert_eq!(
                    updated.get("command").and_then(serde_json::Value::as_str),
                    Some(":")
                );
                assert_eq!(
                    updated.get("timeout_ms").and_then(serde_json::Value::as_u64),
                    Some(1000)
                );
            }
            other => panic!("expected Allow with rewritten input, got {other:?}"),
        }

        assert_eq!(registry.len().await, 1, "monitor should be registered");
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
}
