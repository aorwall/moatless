//! The two status reads every container runtime answers the same way.
//!
//! A runtime's `SandboxStatusReader` impl differs only in `server_logs`. Both
//! reads below resolve the sandbox id and its server config from the plan, then
//! hand the result to the runtime's own resolver through [`StatusResolve`].

use async_trait::async_trait;
use tracing::warn;

use crate::lifecycle::{ProvisionDeps, TaskServers};
use moatless_sandbox_runtime::client::SandboxConnector;
use moatless_sandbox_runtime::error::SandboxError;
use moatless_sandbox_runtime::live_status::LiveStatus;
use moatless_sandbox_runtime::provisioning::{allocated_port, server_capabilities_from_config};
use moatless_sandbox_runtime::schemas::{SandboxStatusFullResponse, ServerEntry};
use moatless_types::sandbox::{SandboxDesiredState, ServerStatus};
use moatless_types::task::TaskStatus;
use moatless_workspace_types::workspace::ServerConfig;

/// A container runtime's own status resolver, as the shared reads call it.
///
/// `connector` is the adapter that owns the resolver — a resolver reaches the
/// running sandbox over gRPC through it rather than holding its own client.
#[async_trait]
pub trait StatusResolve: Send + Sync {
    /// Full sandbox status: the environment, its agent, and every server in
    /// `server_configs`.
    ///
    /// `server_configs` are the canonical server definitions from the database;
    /// the runtime overlays its own container statuses onto them.
    async fn resolve_sandbox_status(
        &self,
        task_id: &str,
        sandbox_id: &str,
        server_configs: &[ServerConfig],
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
        connector: &dyn SandboxConnector,
    ) -> SandboxStatusFullResponse;

    /// Only the Task's live status, omitting servers and diagnostics.
    async fn resolve_live_status(
        &self,
        task_id: &str,
        sandbox_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
        connector: &dyn SandboxConnector,
    ) -> LiveStatus;
}

/// The sandbox id carrying a Task's containers.
///
/// A deployment with no plan wired runs one sandbox per Task under the Task's
/// own id, so `None` is not an error.
pub async fn sandbox_id_for_task(
    plan: Option<&ProvisionDeps>,
    task_id: &str,
) -> Result<String, SandboxError> {
    match plan {
        Some(plan) => plan
            .sandbox_id_for_task(task_id)
            .await
            .map_err(SandboxError::Runtime),
        None => Ok(task_id.to_string()),
    }
}

/// Full status: the sandbox, its agent, and every server the plan declares.
pub async fn sandbox_status(
    resolver: &dyn StatusResolve,
    plan: Option<&ProvisionDeps>,
    connector: &dyn SandboxConnector,
    task_id: &str,
    task_status: TaskStatus,
    desired_state: SandboxDesiredState,
) -> Result<SandboxStatusFullResponse, SandboxError> {
    let sandbox_id = sandbox_id_for_task(plan, task_id).await?;
    let servers = if let Some(plan) = plan {
        plan.task_servers(task_id).await
    } else {
        warn!(task_id, "Sandbox plan repositories unavailable for status");
        TaskServers::default()
    };

    // A headless Sandbox has no sidecars to observe, so the runtime resolver is
    // given nothing to overlay: handed the declared configs it would find no
    // container for any of them and report every one as `Starting`, forever,
    // behind a preview URL nothing is listening on.
    let observable: &[ServerConfig] = if servers.headless {
        &[]
    } else {
        &servers.configs
    };
    let mut status = resolver
        .resolve_sandbox_status(
            task_id,
            &sandbox_id,
            observable,
            task_status,
            desired_state,
            connector,
        )
        .await;
    if servers.headless {
        status.servers = disabled_servers(&servers.configs, task_id);
    }
    Ok(status)
}

/// Why a headless Sandbox is running none of its Workspace's servers, and what
/// to do about it.
const HEADLESS_DETAIL: &str = "This task is headless, so no preview server container was created. \
     Recreate the task with `--headless false` to run it.";

/// The Workspace's declared servers as a headless Sandbox leaves them: named,
/// so a reader sees what the Workspace declares, and `Disabled` with no URL, so
/// nothing waits on a port that was never opened.
fn disabled_servers(configs: &[ServerConfig], task_id: &str) -> Vec<ServerEntry> {
    configs
        .iter()
        .filter_map(|config| {
            Some(ServerEntry {
                name: config.name.clone(),
                label: config.label.clone().unwrap_or_else(|| config.name.clone()),
                port: allocated_port(config, task_id)?,
                status: ServerStatus::Disabled,
                url: None,
                error: None,
                detail: Some(HEADLESS_DETAIL.to_string()),
                default: config.default,
                capabilities: server_capabilities_from_config(config),
            })
        })
        .collect()
}

/// Refuse a log read for a server this Task is not running, naming what it can.
///
/// Without this the read reaches the container runtime, whose missing-container
/// error the provider can only report as `Internal` — a 500 naming neither
/// ordinary cause. `sandbox` is not routed here: it exists on every pod.
pub async fn require_running_server(
    plan: Option<&ProvisionDeps>,
    task_id: &str,
    server_name: &str,
) -> Result<(), SandboxError> {
    let Some(plan) = plan else {
        return Ok(());
    };
    match log_read_refusal(&plan.task_servers(task_id).await, task_id, server_name) {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// The reason a named server has no container to read logs from, or `None` when
/// one is running under that name.
fn log_read_refusal(
    servers: &TaskServers,
    task_id: &str,
    server_name: &str,
) -> Option<SandboxError> {
    if servers.headless {
        return Some(SandboxError::NotFound(format!(
            "Task {task_id} is headless, so no preview server container was created and \
             '{server_name}' has no logs. Recreate the task with `--headless false` to run \
             the servers its workspace declares."
        )));
    }
    if servers.configs.iter().any(|c| c.name == server_name) {
        return None;
    }
    let declared: Vec<&str> = servers.configs.iter().map(|c| c.name.as_str()).collect();
    let alternatives = if declared.is_empty() {
        "Its workspace declares none. A port opened by `moat tasks scripts run` is not a \
         server: its output goes to the terminal the script runs in."
            .to_string()
    } else {
        format!("Declared servers: {}.", declared.join(", "))
    };
    Some(SandboxError::NotFound(format!(
        "Task {task_id} has no server named '{server_name}'. {alternatives} \
         Use `--server sandbox` for the sandbox container's own logs."
    )))
}

/// Display status only — no server config read, so no workspace round-trip.
pub async fn live_status(
    resolver: &dyn StatusResolve,
    plan: Option<&ProvisionDeps>,
    connector: &dyn SandboxConnector,
    task_id: &str,
    task_status: TaskStatus,
    desired_state: SandboxDesiredState,
) -> Result<LiveStatus, SandboxError> {
    let sandbox_id = sandbox_id_for_task(plan, task_id).await?;
    Ok(resolver
        .resolve_live_status(task_id, &sandbox_id, task_status, desired_state, connector)
        .await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_workspace_types::workspace::Port;

    fn server(name: &str, port: u16) -> ServerConfig {
        ServerConfig {
            name: name.to_string(),
            port: Port::new(port),
            ..Default::default()
        }
    }

    fn headless(configs: Vec<ServerConfig>) -> TaskServers {
        TaskServers {
            configs,
            headless: true,
        }
    }

    #[test]
    fn a_headless_task_reports_its_servers_as_disabled_with_no_url() {
        let entries = disabled_servers(&[server("preview", 3200)], "t1");
        let [entry] = &entries[..] else {
            panic!("one declared server, one entry: {entries:?}");
        };
        assert_eq!(entry.name, "preview");
        assert_eq!(entry.port, 3200);
        assert_eq!(entry.status, ServerStatus::Disabled);
        assert_eq!(entry.url, None, "nothing is listening on the port");
        assert_eq!(entry.detail.as_deref(), Some(HEADLESS_DETAIL));
    }

    #[test]
    fn a_headless_log_read_is_refused_by_name_with_the_reason() {
        let refusal = log_read_refusal(&headless(vec![server("preview", 3200)]), "t1", "preview")
            .expect("headless runs no server container");
        let SandboxError::NotFound(message) = refusal else {
            panic!("a container that was never created is not found: {refusal:?}");
        };
        assert!(message.contains("headless"), "{message}");
        assert!(message.contains("--headless false"), "{message}");
    }

    #[test]
    fn an_unknown_server_is_refused_with_the_names_that_would_work() {
        let servers = TaskServers {
            configs: vec![server("dev", 3000), server("api", 8080)],
            headless: false,
        };
        assert!(
            log_read_refusal(&servers, "t1", "dev").is_none(),
            "a declared server on a running sandbox reads its own logs"
        );
        let refusal =
            log_read_refusal(&servers, "t1", "preview").expect("no server is named 'preview'");
        let SandboxError::NotFound(message) = refusal else {
            panic!("an undeclared name is not found: {refusal:?}");
        };
        assert!(message.contains("dev, api"), "{message}");
    }

    #[test]
    fn a_workspace_declaring_no_servers_says_so_rather_than_listing_none() {
        let refusal = log_read_refusal(&TaskServers::default(), "t1", "preview")
            .expect("no server is named 'preview'");
        let SandboxError::NotFound(message) = refusal else {
            panic!("an undeclared name is not found: {refusal:?}");
        };
        assert!(message.contains("declares none"), "{message}");
        assert!(message.contains("scripts run"), "{message}");
    }

    #[test]
    fn a_server_with_no_allocated_port_is_left_out_rather_than_reported_on_port_zero() {
        assert!(disabled_servers(&[server("preview", 0)], "t1").is_empty());
    }
}
