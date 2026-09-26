//! Read-only domain service for resolving sandbox status from the local
//! process registry.

use async_trait::async_trait;

use moatless_sandbox_control::status::StatusResolve;
use moatless_sandbox_runtime::client::{AgentStatusSnapshot, SandboxConnector, fetch_agent_status};
use moatless_sandbox_runtime::live_status::{LiveStatus, derive_live_status};
use moatless_sandbox_runtime::schemas::*;
use moatless_types::sandbox::SandboxDesiredState;
use moatless_types::sandbox::SandboxStatus;
use moatless_types::task::TaskStatus;

use super::client::LocalClient;

/// Read-only domain service mapping the in-memory process registry to
/// `SandboxStatus`.
pub struct LocalStatusResolver {
    client: LocalClient,
}

impl LocalStatusResolver {
    pub fn new(client: LocalClient) -> Self {
        Self { client }
    }

    /// The registry is keyed by the sandbox id `SandboxRuntime` operations
    /// receive; a shared sandbox's member task has none of its own.
    async fn inspect(&self, sandbox_id: &str) -> SandboxStatus {
        match self.client.snapshot(sandbox_id).await {
            Some(snapshot) if snapshot.alive => SandboxStatus::Ready,
            Some(_) => SandboxStatus::Stopped,
            None => SandboxStatus::NotCreated,
        }
    }
}

#[async_trait]
impl StatusResolve for LocalStatusResolver {
    async fn resolve_sandbox_status(
        &self,
        task_id: &str,
        sandbox_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
        connector: &dyn SandboxConnector,
    ) -> SandboxStatusFullResponse {
        let sandbox_status = self.inspect(sandbox_id).await;
        let is_running = sandbox_status == SandboxStatus::Ready;

        let (agent_status, scheduled_jobs, commands) = if is_running {
            let AgentStatusSnapshot {
                status,
                scheduled_jobs,
                commands,
            } = fetch_agent_status(task_id, connector).await;
            (status, scheduled_jobs, commands)
        } else {
            (None, Vec::new(), Vec::new())
        };

        let live_status = derive_live_status(
            task_status,
            desired_state,
            sandbox_status,
            None,
            agent_status,
        );

        SandboxStatusFullResponse {
            sandbox_status: live_status.sandbox_status,
            sandbox_error: live_status.sandbox_error,
            agent_status: live_status.agent_status,
            resources: None,
            container_diagnostics: None,
            runtime_events: None,
            scheduled_jobs,
            commands,
            sandbox_desired_state: None,
            current_sandbox_image: None,
            latest_sandbox_image: None,
            image_up_to_date: None,
        }
    }

    async fn resolve_live_status(
        &self,
        task_id: &str,
        sandbox_id: &str,
        task_status: TaskStatus,
        desired_state: SandboxDesiredState,
        connector: &dyn SandboxConnector,
    ) -> LiveStatus {
        let sandbox_status = self.inspect(sandbox_id).await;
        let (agent_status, commands) = if sandbox_status == SandboxStatus::Ready {
            let snapshot = fetch_agent_status(task_id, connector).await;
            let commands = snapshot
                .commands
                .iter()
                .map(ManagedCommand::summary)
                .collect();
            (snapshot.status, commands)
        } else {
            (None, Vec::new())
        };

        derive_live_status(
            task_status,
            desired_state,
            sandbox_status,
            None,
            agent_status,
        )
        .with_commands(commands)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_ports::sandbox as core_sandbox;
    use moatless_sandbox_runtime::client::SandboxClient;

    struct NoSandbox;

    #[async_trait]
    impl SandboxConnector for NoSandbox {
        async fn sandbox_client(
            &self,
            _task_id: &str,
        ) -> Result<SandboxClient, core_sandbox::SandboxError> {
            Err(core_sandbox::SandboxError::Other(
                "unreachable in this test".to_string(),
            ))
        }
    }

    #[tokio::test]
    async fn a_member_task_reads_its_shared_sandbox() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("sandbox.sh");
        std::fs::write(&bin, "#!/bin/sh\nexec sleep 300\n").unwrap();
        std::fs::set_permissions(&bin, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let client = LocalClient::new(String::new(), bin.display().to_string());
        let env = [("PATH".to_string(), std::env::var("PATH").unwrap())];
        client
            .spawn(
                "lead-task",
                &env,
                &dir.path().join("sandbox.log"),
                0,
                0,
                None,
            )
            .await
            .unwrap();
        let resolver = LocalStatusResolver::new(client.clone());

        let status = resolver
            .resolve_live_status(
                "member-task",
                "lead-task",
                TaskStatus::Open,
                SandboxDesiredState::Running,
                &NoSandbox,
            )
            .await;
        client.purge("lead-task").await;

        assert_eq!(
            status.sandbox_status,
            SandboxStatus::Ready,
            "a member task must see the sandbox it shares, not its own absent one"
        );
    }
}
