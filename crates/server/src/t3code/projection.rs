//! Outbound projection from the backend's sandbox vocabulary to the T3 contract.
//!
//! The backend's status vocabulary is deliberately wider than the T3 contract's:
//! `crate::sandbox::schemas::SandboxStatusFullResponse` carries thirteen fields
//! and the contract's carries two. These functions narrow the backend types to
//! the contract's, so the T3 surface is only ever handed the smaller vocabulary
//! it was designed for. Widening the contract by unifying the types would hand a
//! client a surface nobody chose to give it, so the narrowing lives here as its
//! own named projection rather than inside a port implementation.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt as _;
use moatless_ports::sandbox as core_sandbox;

use crate::sandbox::access as backend_sandbox;
use crate::sandbox::error::SandboxError;
use crate::sandbox::status_reader::SandboxStatusReader;

/// The T3 contract observer, built from the backend's rich status read.
///
/// The HTTP routes read the thirteen-field response directly through
/// [`SandboxStatusReader`]; T3 only ever sees the two-field contract shape the
/// functions below narrow to. This is the single seam where that narrowing
/// happens, so the projection sits beside the contract rather than being
/// repeated inside each provider's port implementation.
/// `turns` is the Task band's half of the running rule. It is folded into
/// `live_status` here, at the composition root, so T3's sandbox panel and the
/// REST live-status endpoint answer "is the agent working" from the same rule
/// — the runtime reader below can only see the pod and the heartbeat.
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
        task_status: moatless_tasks::task::models::TaskStatus,
        desired_state: moatless_tasks::task::models::SandboxDesiredState,
    ) -> Result<core_sandbox::ContractSandboxStatus, core_sandbox::SandboxError> {
        self.reader
            .sandbox_status(task_id, task_status, desired_state)
            .await
            .map(sandbox_status)
            .map_err(SandboxError::into_contract)
    }

    async fn live_status(
        &self,
        task_id: &str,
        task_status: moatless_tasks::task::models::TaskStatus,
        desired_state: moatless_tasks::task::models::SandboxDesiredState,
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

    async fn server_logs(
        &self,
        task_id: &str,
        server_name: &str,
        options: core_sandbox::LogOptions,
    ) -> Result<core_sandbox::ServerLogStream, core_sandbox::SandboxError> {
        self.reader
            .server_logs(task_id, server_name, log_options(options))
            .await
            .map(|stream| {
                stream
                    .map(|item| item.map_err(SandboxError::into_contract))
                    .boxed()
            })
            .map_err(SandboxError::into_contract)
    }
}

pub(crate) fn sandbox_status(
    response: crate::sandbox::schemas::SandboxStatusFullResponse,
) -> core_sandbox::ContractSandboxStatus {
    core_sandbox::ContractSandboxStatus {
        sandbox_status: response.sandbox_status,
        servers: response.servers.into_iter().map(server_entry).collect(),
    }
}

pub(crate) fn live_status(
    status: crate::sandbox::live_status::LiveStatus,
) -> core_sandbox::LiveStatus {
    core_sandbox::LiveStatus {
        sandbox_status: status.sandbox_status,
        agent_status: status.agent_status.map(|status| match status {
            crate::sandbox::live_status::AgentStatus::Running => core_sandbox::AgentStatus::Running,
            crate::sandbox::live_status::AgentStatus::Waiting => core_sandbox::AgentStatus::Waiting,
            crate::sandbox::live_status::AgentStatus::Idle => core_sandbox::AgentStatus::Idle,
            crate::sandbox::live_status::AgentStatus::Unknown => core_sandbox::AgentStatus::Unknown,
        }),
        sandbox_error: status.sandbox_error,
        // Carried, not narrowed away: the agent reports `idle` while a `moat
        // cmd` command runs, so without these T3 shows a Task mid-build as one
        // with nothing left to do. `CommandSummary` is already the narrow half
        // of the command shape — the status-read response keeps the wide one.
        commands: status.commands,
    }
}

fn server_entry(entry: crate::sandbox::schemas::ServerEntry) -> core_sandbox::ServerEntry {
    core_sandbox::ServerEntry {
        name: entry.name,
        label: entry.label,
        port: entry.port,
        status: match entry.status {
            moatless_types::sandbox::ServerStatus::Stopped => core_sandbox::ServerStatus::Stopped,
            moatless_types::sandbox::ServerStatus::Installing => {
                core_sandbox::ServerStatus::Installing
            }
            moatless_types::sandbox::ServerStatus::Starting => core_sandbox::ServerStatus::Starting,
            moatless_types::sandbox::ServerStatus::Started => core_sandbox::ServerStatus::Started,
            moatless_types::sandbox::ServerStatus::Failed => core_sandbox::ServerStatus::Failed,
        },
        url: entry.url,
        error: entry.error,
        detail: entry.detail,
        default: entry.default,
        capabilities: core_sandbox::ServerCapabilities {
            address_bar: entry.capabilities.address_bar,
            inspector: entry.capabilities.inspector,
            reload: entry.capabilities.reload,
            open_external: entry.capabilities.open_external,
            viewport: entry.capabilities.viewport,
            console: entry.capabilities.console,
        },
    }
}

pub(crate) fn log_options(options: core_sandbox::LogOptions) -> backend_sandbox::LogOptions {
    backend_sandbox::LogOptions {
        previous: options.previous,
        follow: options.follow,
        tail_lines: options.tail_lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::live_status::AgentStatus;
    use crate::sandbox::schemas::{
        CommandSummary, ContainerDiagnostics, ManagedCommand, ManagedCommandState, RuntimeEvent,
        SandboxStatusFullResponse, ScheduledJobInfo, ScheduledJobKind, ServerEntry, WakeupJobInfo,
    };
    use moatless_api_schemas::enums::SandboxDesiredState;
    use moatless_api_schemas::sandbox::ResourceMetrics;
    use moatless_types::sandbox::SandboxStatus;
    use moatless_types::sandbox::{ServerCapabilities, ServerStatus};

    /// Every field of the rich response populated with a value distinguishable
    /// from a default, written as an exhaustive struct literal: a field added to
    /// the rich type stops this compiling until someone decides whether the T3
    /// contract should see it.
    fn fully_populated_rich_status() -> SandboxStatusFullResponse {
        SandboxStatusFullResponse {
            sandbox_status: SandboxStatus::Ready,
            sandbox_error: Some("a stale image warning".to_string()),
            agent_status: Some(AgentStatus::Waiting),
            servers: vec![fully_populated_rich_server()],
            resources: Some(ResourceMetrics {
                memory_usage_mb: 812.0,
                memory_limit_mb: Some(4096.0),
                memory_usage_percent: Some(19.8),
                cpu_usage_percent: Some(37.5),
                event_loop_latency_ms: 2.5,
                uptime_seconds: 903.0,
            }),
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
            latest_sandbox_image: Some("moatless/sandbox:v2".to_string()),
            image_up_to_date: Some(false),
        }
    }

    fn fully_populated_rich_server() -> ServerEntry {
        ServerEntry {
            name: "frontend".to_string(),
            label: "Frontend".to_string(),
            port: 5173,
            status: ServerStatus::Started,
            url: Some("https://sandbox.example/preview".to_string()),
            error: Some("last start exited 1".to_string()),
            detail: Some("vite ready in 412ms".to_string()),
            default: true,
            capabilities: ServerCapabilities {
                address_bar: true,
                inspector: true,
                reload: true,
                open_external: true,
                viewport: true,
                console: true,
            },
        }
    }

    /// The T3 contract is deliberately narrower than the backend's status
    /// vocabulary. Both halves of that narrowing are pinned structurally: the
    /// exhaustive literal above fails to compile when the rich type gains a
    /// field, and the exhaustive destructuring below fails to compile when the
    /// contract type does — so neither side widens without a decision.
    #[test]
    fn the_projected_status_carries_exactly_the_contract_field_set() {
        let rich = fully_populated_rich_status();
        let core_sandbox::ContractSandboxStatus {
            sandbox_status,
            servers,
        } = sandbox_status(rich);

        assert_eq!(sandbox_status, SandboxStatus::Ready);
        assert_eq!(servers.len(), 1);
    }

    #[test]
    fn the_projected_server_entry_carries_exactly_the_contract_field_set() {
        let core_sandbox::ServerEntry {
            name,
            label,
            port,
            status,
            url,
            error,
            detail,
            default,
            capabilities,
        } = server_entry(fully_populated_rich_server());

        assert_eq!(name, "frontend");
        assert_eq!(label, "Frontend");
        assert_eq!(port, 5173);
        assert_eq!(status, core_sandbox::ServerStatus::Started);
        assert_eq!(url.as_deref(), Some("https://sandbox.example/preview"));
        assert_eq!(error.as_deref(), Some("last start exited 1"));
        assert_eq!(detail.as_deref(), Some("vite ready in 412ms"));
        assert!(default);

        let core_sandbox::ServerCapabilities {
            address_bar,
            inspector,
            reload,
            open_external,
            viewport,
            console,
        } = capabilities;
        assert!(address_bar && inspector && reload && open_external && viewport && console);
    }

    /// `live_status` is an error-only collapse — the field sets already match —
    /// so what it pins is that every field still crosses, rather than a
    /// narrowing.
    #[test]
    fn the_projected_live_status_carries_exactly_the_contract_field_set() {
        let rich = crate::sandbox::live_status::LiveStatus {
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

    #[test]
    fn the_projected_log_options_carry_exactly_the_backend_field_set() {
        let backend_sandbox::LogOptions {
            previous,
            follow,
            tail_lines,
        } = log_options(core_sandbox::LogOptions {
            previous: true,
            follow: true,
            tail_lines: 250,
        });

        assert!(previous);
        assert!(follow);
        assert_eq!(tail_lines, 250);
    }
}
