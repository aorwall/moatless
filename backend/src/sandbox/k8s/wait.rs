//! Shared bounded polling for strict Kubernetes resource absence.

use std::future::Future;
use std::time::Duration;

use tracing::{debug, warn};

use crate::sandbox::error::SandboxError;

pub async fn wait_absent<F, Fut, E>(
    task_id: &str,
    resource: &str,
    timeout: Duration,
    mut observe_absent: F,
) -> Result<(), SandboxError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<bool, E>>,
    E: std::fmt::Display,
{
    const POLL_INTERVAL: Duration = Duration::from_secs(2);

    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match observe_absent().await {
            Ok(true) => return Ok(()),
            Ok(false) => debug!(task_id, resource, "Waiting for Kubernetes resource deletion"),
            Err(error) => {
                warn!(task_id, resource, error = %error, "Failed to observe resource deletion; retrying");
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(SandboxError::NotReady(format!(
                "Timed out after {}s waiting for task {task_id} {resource} deletion",
                timeout.as_secs()
            )));
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}
