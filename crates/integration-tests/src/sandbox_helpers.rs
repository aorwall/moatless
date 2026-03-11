//! Sandbox polling helpers for integration tests.
//!
//! All helpers poll `/api/sandbox/v1/tasks/{task_id}/status` at 2-second
//! intervals until the desired state is reached or `timeout` expires.

use std::time::Duration;

use moatless_api_client::types::{AgentState, SandboxStatus};
use moatless_cli::client::MoatClient;

/// Poll until `environment_status == "ready"` and `sandbox_ready == true`.
/// Includes a 3-second stability re-check.
pub async fn wait_for_sandbox_ready(client: &MoatClient, task_id: &str, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(resp) = client.sandbox_status_handler(task_id).await {
            let status = resp.into_inner();
            if status.environment_status == SandboxStatus::Ready && status.sandbox_ready {
                // Stability re-check after 3s
                tokio::time::sleep(Duration::from_secs(3)).await;
                if let Ok(resp2) = client.sandbox_status_handler(task_id).await {
                    let s2 = resp2.into_inner();
                    if s2.environment_status == SandboxStatus::Ready && s2.sandbox_ready {
                        return true;
                    }
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Poll until `environment_status` matches `target`.
pub async fn wait_for_sandbox_status(
    client: &MoatClient,
    task_id: &str,
    target: SandboxStatus,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(resp) = client.sandbox_status_handler(task_id).await {
            if resp.into_inner().environment_status == target {
                return true;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Poll until `agent_state == "idle"`.
pub async fn wait_for_agent_idle(
    client: &MoatClient,
    task_id: &str,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(resp) = client.sandbox_status_handler(task_id).await {
            if resp.into_inner().agent_state == AgentState::Idle {
                return true;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Poll until `list_files_handler` returns a non-empty file list.
pub async fn wait_for_files_available(
    client: &MoatClient,
    task_id: &str,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(resp) = client.list_files_handler(task_id, None, None).await {
            if !resp.into_inner().files.is_empty() {
                return true;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
