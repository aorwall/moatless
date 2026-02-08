//! Fire-and-forget notification to sandbox pods.

/// Notify a sandbox pod that new messages are available.
///
/// Fire-and-forget: logs warnings on failure, never propagates errors.
pub async fn notify_sandbox(namespace: &str, service_name: &str) {
    let url = format!("http://{service_name}.{namespace}:8765/notify-messages");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    match client.post(&url).send().await {
        Ok(_) => {
            tracing::debug!(url = %url, "Notified sandbox pod");
        }
        Err(e) => {
            tracing::warn!(url = %url, error = %e, "Failed to notify sandbox pod");
        }
    }
}
