//! Sandbox polling helpers for integration tests.
//!
//! All helpers poll `/api/sandbox/v1/tasks/{task_id}/status` at 2-second
//! intervals until the desired state is reached or `timeout` expires.

use std::time::Duration;

use moatless_api_schemas::sandbox::{AgentStatus, SandboxStatus};
use moatless_cli::client::MoatClient;

/// Poll until `sandbox_status == "ready"`.
/// Includes a 3-second stability re-check.
pub async fn wait_for_sandbox_ready(client: &MoatClient, task_id: &str, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(status) = client.api().sandbox_status(task_id).await {
            if status.sandbox_status == SandboxStatus::Ready {
                // Stability re-check after 3s
                tokio::time::sleep(Duration::from_secs(3)).await;
                if let Ok(s2) = client.api().sandbox_status(task_id).await {
                    if s2.sandbox_status == SandboxStatus::Ready {
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

/// Poll until `sandbox_status` matches `target`.
pub async fn wait_for_sandbox_status(
    client: &MoatClient,
    task_id: &str,
    target: &str,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(status) = client.api().sandbox_status(task_id).await {
            if status.sandbox_status.as_str() == target {
                return true;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Poll until `agent_status == "idle"`.
pub async fn wait_for_agent_idle(client: &MoatClient, task_id: &str, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(status) = client.api().sandbox_status(task_id).await {
            if status.agent_status == Some(AgentStatus::Idle) {
                return true;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Poll until `list_files` returns a non-empty file list.
pub async fn wait_for_files_available(
    client: &MoatClient,
    task_id: &str,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(resp) = client.api().list_files(task_id).await {
            if !resp.files.is_empty() {
                return true;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

/// Poll until `sandbox_status` contains an error or error message is set.
pub async fn wait_for_sandbox_error(
    client: &MoatClient,
    task_id: &str,
    timeout: Duration,
) -> Option<String> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Ok(status) = client.api().sandbox_status(task_id).await {
            if status.sandbox_status == SandboxStatus::Error {
                return Some(status.sandbox_status.to_string());
            }
            if let Some(ref error) = status.sandbox_error {
                if !error.is_empty() {
                    return Some(error.clone());
                }
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
