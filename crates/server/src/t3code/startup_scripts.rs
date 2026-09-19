//! Starting a Task's `run_on_start` Scripts — its Workspace's and its own —
//! when the Sandbox reports Ready.
//!
//! The trigger half of [`StartupScripts`]: the Sandbox becoming Ready is the
//! event, and both runtimes publish it once per operation generation
//! (`moatless_sandbox_runtime::observation_events`), so a Task's flagged
//! Scripts start once per Sandbox start, on one replica.

use std::sync::Arc;

use moatless_core::events::EventPublisher;
use moatless_loops::adapters::events::TaskEventSubscriber;
use moatless_t3code::StartupScripts;
use moatless_tasks::agent_event::AgentEvent;
use moatless_types::sandbox::{SandboxEventPayload, SandboxStatus};
use tracing::warn;

/// The `lifecycleEvent` a failed startup Script is reported under — both when
/// publishing it and when refusing to react to it, which is why it is one
/// constant.
const STARTUP_SCRIPT_FAILED: &str = "startup_script_failed";

/// Runs a Task's flagged Scripts on `sandbox.ready`.
pub struct StartupScriptSubscriber {
    scripts: StartupScripts,
    events: Option<Arc<dyn EventPublisher>>,
}

impl StartupScriptSubscriber {
    pub fn new(scripts: StartupScripts, events: Option<Arc<dyn EventPublisher>>) -> Self {
        Self { scripts, events }
    }
}

/// Whether this Ready event is one to start Scripts for.
///
/// A failed Script also reports as Ready — the Sandbox is ready even if the
/// Script is not — so left unguarded, a Script that cannot start re-triggers
/// itself forever.
fn is_own_failure_report(event: &AgentEvent) -> bool {
    event
        .payload
        .get("lifecycleEvent")
        .and_then(serde_json::Value::as_str)
        == Some(STARTUP_SCRIPT_FAILED)
}

#[async_trait::async_trait]
impl TaskEventSubscriber for StartupScriptSubscriber {
    fn name(&self) -> &'static str {
        "startup-scripts"
    }

    async fn on_task_event(&self, event: &AgentEvent) {
        if is_own_failure_report(event) {
            return;
        }
        let scripts = self.scripts.clone();
        let events = self.events.clone();
        let task_id = event.task_id.clone();
        tokio::spawn(async move {
            // reason: a Sandbox becoming Ready is the runtime's observation and
            // has no user behind it to scope the Workspace read to.
            #[allow(clippy::disallowed_methods)]
            let viewer = moatless_types::access_control::ScopeContext::system();

            let failures = match scripts.run_for_task(&viewer, &task_id).await {
                Ok(failures) => failures,
                Err(error) => {
                    warn!(
                        task_id,
                        %error,
                        "Startup scripts were not read; none were started"
                    );
                    return;
                }
            };
            for failure in failures {
                let message = format!(
                    "Startup script \"{}\" did not start: {}",
                    failure.name, failure.error
                );
                warn!(task_id, script = %failure.name, error = %failure.error, "{message}");
                if let Some(events) = &events {
                    events
                        .publish_sandbox_event_with_payload(
                            &task_id,
                            SandboxStatus::Ready,
                            &SandboxEventPayload {
                                message,
                                lifecycle_event: Some(STARTUP_SCRIPT_FAILED.to_string()),
                                error: Some(failure.error.to_string()),
                                ..Default::default()
                            },
                        )
                        .await;
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use moatless_types::event::EventType;
    use serde_json::json;

    use super::*;

    fn ready_event(payload: serde_json::Value) -> AgentEvent {
        AgentEvent {
            event_id: "sandbox:task-1:1:ready".to_string(),
            event_type: EventType::Sandbox,
            sub_type: SandboxStatus::Ready.as_str().to_string(),
            task_id: "task-1".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            is_error: false,
            agent_type: None,
            turn_number: None,
            payload,
        }
    }

    #[test]
    fn a_runtime_ready_event_starts_scripts() {
        assert!(!is_own_failure_report(&ready_event(json!({}))));
        assert!(!is_own_failure_report(&ready_event(
            json!({ "lifecycleEvent": "pod_ready" })
        )));
    }

    #[test]
    fn this_subscribers_own_failure_report_does_not_start_scripts() {
        assert!(is_own_failure_report(&ready_event(
            json!({ "lifecycleEvent": STARTUP_SCRIPT_FAILED })
        )));
    }
}
