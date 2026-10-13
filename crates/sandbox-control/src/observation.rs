//! The [`core_sandbox::SandboxObserver`] port, answered from a runtime's
//! [`SandboxStatusReader`].
//!
//! `SandboxStatusFullResponse` carries thirteen fields; the contract exposes
//! only what each caller needs. [`sandbox_status`] narrows to the two-field
//! status indicator shape, [`sandbox_detail`] to the wider sandbox-panel shape
//! (image, containers, runtime events) — kept as separate named functions so
//! widening one cannot widen the other.

use std::sync::Arc;

use async_trait::async_trait;
use moatless_ports::sandbox as core_sandbox;

use moatless_sandbox_runtime::error::SandboxError;
use moatless_sandbox_runtime::status_reader::SandboxStatusReader;
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::task::TaskStatus;

/// The [`core_sandbox::SandboxObserver`], built from the runtime's rich status read.
///
/// `turns` folds into `live_status` here so T3's panel and the REST endpoint
/// answer "is the agent working" from the same rule; the reader alone only sees
/// the pod and heartbeat.
pub fn contract_observer(
    reader: Arc<dyn SandboxStatusReader>,
    turns: Option<Arc<dyn moatless_ports::task::TurnStateReader>>,
) -> Arc<dyn core_sandbox::SandboxObserver> {
    Arc::new(ContractObserver { reader, turns })
}

struct ContractObserver {
    reader: Arc<dyn SandboxStatusReader>,
    turns: Option<Arc<dyn moatless_ports::task::TurnStateReader>>,
}

#[async_trait]
impl core_sandbox::SandboxObserver for ContractObserver {
    async fn sandbox_status(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<core_sandbox::ContractSandboxStatus, core_sandbox::SandboxError> {
        self.reader
            .sandbox_status(task_id, task_status, desired_state)
            .await
            .map(sandbox_status)
            .map_err(SandboxError::into_contract)
    }

    async fn detail(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<core_sandbox::SandboxDetail, core_sandbox::SandboxError> {
        self.reader
            .sandbox_status(task_id, task_status, desired_state)
            .await
            .map(sandbox_detail)
            .map_err(SandboxError::into_contract)
    }

    async fn live_status(
        &self,
        task_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
    ) -> Result<core_sandbox::LiveStatus, core_sandbox::SandboxError> {
        let status = self
            .reader
            .live_status(task_id, task_status, desired_state)
            .await
            .map(live_status)
            .map_err(SandboxError::into_contract)?;

        // A failed turn read degrades to the runtime's own answer rather than
        // to an error, exactly as the REST handler does: a status call that
        // fails is worse than one that is momentarily as narrow as it was.
        let Some(turns) = self.turns.as_deref() else {
            return Ok(status);
        };
        let signals = match turns.turn_signals(task_id).await {
            Ok(signals) => signals,
            Err(e) => {
                tracing::warn!(task_id, error = %e, "live status: turn signals unavailable");
                return Ok(status);
            }
        };

        // The stop reason is not known here, and does not need to be: it only
        // splits `Stopped` from `Error`, and both leave the agent field alone.
        Ok(moatless_sandbox_runtime::live_status::apply_running_rule(
            status,
            desired_state,
            None,
            signals,
        ))
    }
}

pub(crate) fn sandbox_status(
    response: moatless_sandbox_runtime::schemas::SandboxStatusFullResponse,
) -> core_sandbox::ContractSandboxStatus {
    core_sandbox::ContractSandboxStatus {
        sandbox_status: response.sandbox_status,
    }
}

pub(crate) fn sandbox_detail(
    response: moatless_sandbox_runtime::schemas::SandboxStatusFullResponse,
) -> core_sandbox::SandboxDetail {
    core_sandbox::SandboxDetail {
        // `latest_sandbox_image` and `image_up_to_date` stay behind: every
        // status reader leaves them `None` and the REST handler fills them
        // afterwards from the lifecycle orchestrator, which this read does not
        // go through. `SandboxOperator::expected_image` is T3's half of that.
        current_image: response.current_sandbox_image,
        containers: response
            .container_diagnostics
            .unwrap_or_default()
            .into_iter()
            .map(container)
            .collect(),
        runtime_events: response
            .runtime_events
            .unwrap_or_default()
            .into_iter()
            .map(runtime_event)
            .collect(),
    }
}

fn container(
    diagnostics: moatless_sandbox_runtime::schemas::ContainerDiagnostics,
) -> core_sandbox::SandboxContainer {
    core_sandbox::SandboxContainer {
        name: diagnostics.name,
        state: diagnostics.state,
        restart_count: diagnostics.restart_count,
        exit_code: diagnostics.exit_code,
        terminated_reason: diagnostics.terminated_reason,
        last_exit_code: diagnostics.last_exit_code,
        last_terminated_reason: diagnostics.last_terminated_reason,
        waiting_reason: diagnostics.waiting_reason,
        cpu_millicores: diagnostics.cpu_millicores,
        memory_mb: diagnostics.memory_mb,
    }
}

fn runtime_event(
    event: moatless_sandbox_runtime::schemas::RuntimeEvent,
) -> core_sandbox::SandboxRuntimeEvent {
    core_sandbox::SandboxRuntimeEvent {
        reason: event.reason,
        message: event.message,
        severity: event.severity,
        count: event.count,
        last_timestamp: event.last_timestamp,
    }
}

pub(crate) fn live_status(
    status: moatless_sandbox_runtime::live_status::LiveStatus,
) -> core_sandbox::LiveStatus {
    core_sandbox::LiveStatus {
        sandbox_status: status.sandbox_status,
        agent_status: status.agent_status.map(|status| match status {
            moatless_sandbox_runtime::live_status::AgentStatus::Running => {
                core_sandbox::AgentStatus::Running
            }
            moatless_sandbox_runtime::live_status::AgentStatus::Waiting => {
                core_sandbox::AgentStatus::Waiting
            }
            moatless_sandbox_runtime::live_status::AgentStatus::Idle => {
                core_sandbox::AgentStatus::Idle
            }
            moatless_sandbox_runtime::live_status::AgentStatus::Unknown => {
                core_sandbox::AgentStatus::Unknown
            }
        }),
        sandbox_error: status.sandbox_error,
        // Carried, not narrowed away: the agent reports `idle` while a `moat
        // cmd` command runs, so without these T3 shows a Task mid-build as one
        // with nothing left to do. `CommandSummary` is already the narrow half
        // of the command shape — the status-read response keeps the wide one.
        commands: status.commands,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_sandbox_runtime::live_status::AgentStatus;
    use moatless_sandbox_runtime::schemas::{
        CommandSummary, ContainerDiagnostics, ManagedCommand, ManagedCommandState, RuntimeEvent,
        SandboxStatusFullResponse, ScheduledJobInfo, ScheduledJobKind, WakeupJobInfo,
    };
    use moatless_types::sandbox::SandboxStatus;

    /// Every field populated with a distinguishable value, as an exhaustive
    /// struct literal: a new field on the rich type breaks this until someone
    /// decides whether T3 sees it.
    fn fully_populated_rich_status() -> SandboxStatusFullResponse {
        SandboxStatusFullResponse {
            sandbox_status: SandboxStatus::Ready,
            sandbox_error: Some("a stale image warning".to_string()),
            agent_status: Some(AgentStatus::Waiting),
            // `None` rather than a value, deliberately, on the three fields no
            // writer of this response ever sets: `resources` in all four
            // status resolvers, and the two image fields the REST handler
            // fills after the reader returns. Populating them here would let a
            // projection assert over a shape production never produces.
            resources: None,
            container_diagnostics: Some(vec![ContainerDiagnostics {
                name: "sandbox".to_string(),
                state: "running".to_string(),
                restart_count: 2,
                exit_code: Some(0),
                terminated_reason: Some("Completed".to_string()),
                last_exit_code: Some(137),
                last_terminated_reason: Some("OOMKilled".to_string()),
                waiting_reason: Some("CrashLoopBackOff".to_string()),
                cpu_millicores: Some(450),
                memory_mb: Some(812),
            }]),
            runtime_events: Some(vec![RuntimeEvent {
                reason: "BackOff".to_string(),
                message: "back-off restarting failed container".to_string(),
                severity: Some("warning".to_string()),
                count: Some(3),
                last_timestamp: Some("2026-08-03T10:15:00Z".to_string()),
            }]),
            scheduled_jobs: vec![ScheduledJobInfo {
                id: "job-1".to_string(),
                registered_at_unix_ms: 1_754_000_000_000,
                kind: ScheduledJobKind::Wakeup(WakeupJobInfo {
                    reason: "watching CI".to_string(),
                    prompt_preview: "check the deploy".to_string(),
                    wakeup_at_unix_ms: 1_754_000_600_000,
                }),
            }],
            commands: vec![ManagedCommand {
                id: "cmd-1".to_string(),
                command: "mvn verify".to_string(),
                label: "reactor build".to_string(),
                pid: 4321,
                started_at_unix_ms: 1_754_000_000_000,
                timeout_secs: 3600,
                deadline_unix_ms: 1_754_003_600_000,
                state: ManagedCommandState::Running,
                exit_code: None,
                log_path: "/tmp/moat-commands/task-1/cmd-1.log".to_string(),
                finished_at_unix_ms: 0,
            }],
            sandbox_desired_state: Some(SandboxDesiredState::Running),
            current_sandbox_image: Some("moatless/sandbox:v1".to_string()),
            latest_sandbox_image: None,
            image_up_to_date: None,
        }
    }

    /// The T3 contract is deliberately narrower than the backend's status
    /// vocabulary.
    ///
    /// Both sides are pinned structurally: the literal above and the
    /// destructuring below each fail to compile on a new field, so neither
    /// widens silently.
    #[test]
    fn the_projected_status_carries_exactly_the_contract_field_set() {
        let rich = fully_populated_rich_status();
        let core_sandbox::ContractSandboxStatus { sandbox_status } = sandbox_status(rich);

        assert_eq!(sandbox_status, SandboxStatus::Ready);
    }

    /// The panel's projection is the wide one, and the same exhaustive literal
    /// pins it: a field added to the rich response has to be decided for this
    /// projection as well as for the narrow one above.
    #[test]
    fn the_projected_detail_carries_what_the_sandbox_panel_shows() {
        let core_sandbox::SandboxDetail {
            current_image,
            containers,
            runtime_events,
        } = sandbox_detail(fully_populated_rich_status());

        assert_eq!(current_image.as_deref(), Some("moatless/sandbox:v1"));

        assert_eq!(containers.len(), 1);
        assert_eq!(containers[0].name, "sandbox");
        assert_eq!(containers[0].restart_count, 2);
        assert_eq!(containers[0].cpu_millicores, Some(450));
        assert_eq!(containers[0].memory_mb, Some(812));
        assert_eq!(containers[0].last_exit_code, Some(137));
        assert_eq!(
            containers[0].last_terminated_reason.as_deref(),
            Some("OOMKilled")
        );
        assert_eq!(
            containers[0].waiting_reason.as_deref(),
            Some("CrashLoopBackOff")
        );

        assert_eq!(runtime_events.len(), 1);
        assert_eq!(runtime_events[0].reason, "BackOff");
        assert_eq!(runtime_events[0].severity.as_deref(), Some("warning"));
        assert_eq!(runtime_events[0].count, Some(3));
    }

    /// The panel hides a section it has no answer for, so "no sandbox" and "no
    /// diagnostics" have to arrive as absence rather than as zeroes. This is
    /// the shape a docker-compose deployment actually produces.
    #[test]
    fn a_runtime_that_reports_nothing_projects_absent_sections() {
        let mut rich = fully_populated_rich_status();
        rich.current_sandbox_image = None;
        rich.container_diagnostics = None;
        rich.runtime_events = None;

        let detail = sandbox_detail(rich);
        assert!(detail.current_image.is_none());
        assert!(detail.containers.is_empty());
        assert!(detail.runtime_events.is_empty());
    }

    /// `live_status` is an error-only collapse — the field sets already match —
    /// so what it pins is that every field still crosses, rather than a
    /// narrowing.
    #[test]
    fn the_projected_live_status_carries_exactly_the_contract_field_set() {
        let rich = moatless_sandbox_runtime::live_status::LiveStatus {
            sandbox_status: SandboxStatus::Error,
            agent_status: Some(AgentStatus::Running),
            sandbox_error: Some("image pull failed".to_string()),
            commands: vec![CommandSummary {
                id: "cmd-1".to_string(),
                label: "reactor build".to_string(),
                state: ManagedCommandState::Running,
                started_at_unix_ms: 1_754_000_000_000,
                deadline_unix_ms: 1_754_003_600_000,
                exit_code: None,
            }],
        };
        let core_sandbox::LiveStatus {
            sandbox_status,
            agent_status,
            sandbox_error,
            commands,
        } = live_status(rich);

        assert_eq!(sandbox_status, SandboxStatus::Error);
        assert_eq!(agent_status, Some(core_sandbox::AgentStatus::Running));
        assert_eq!(sandbox_error.as_deref(), Some("image pull failed"));
        assert_eq!(
            commands
                .iter()
                .map(|c| c.label.as_str())
                .collect::<Vec<_>>(),
            ["reactor build"]
        );
    }
}
