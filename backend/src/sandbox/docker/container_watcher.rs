//! Docker container lifecycle watcher.
//!
//! Watches Docker engine events for sandbox containers (identified by the
//! `created-by=moatless` label) and publishes lifecycle events to the event
//! system. Analogous to `k8s/pod_watcher.rs` but for Docker mode.

use std::collections::HashMap;
use std::sync::Arc;

use bollard::query_parameters::EventsOptionsBuilder;
use futures::StreamExt;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::client::DockerClient;
use crate::events::EventPublisher;

/// Spawn a background task that watches Docker container events and publishes
/// lifecycle events to the event system.
///
/// Watches for containers with the `created-by=moatless` label and maps
/// Docker events to sandbox lifecycle events.
pub fn spawn_container_lifecycle_watcher(
    client: DockerClient,
    event_publisher: Arc<dyn EventPublisher>,
    cancel: CancellationToken,
    max_crash_restart_count: i32,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!("Docker container lifecycle watcher started");

        loop {
            // Set up event filters: only watch containers with our label
            let mut filters = HashMap::new();
            filters.insert("type".to_string(), vec!["container".to_string()]);
            filters.insert("label".to_string(), vec!["created-by=moatless".to_string()]);
            filters.insert(
                "event".to_string(),
                vec![
                    "start".to_string(),
                    "die".to_string(),
                    "stop".to_string(),
                    "destroy".to_string(),
                    "oom".to_string(),
                ],
            );

            let opts = EventsOptionsBuilder::new().filters(&filters).build();

            let mut event_stream = client.docker.events(Some(opts));

            loop {
                tokio::select! {
                    () = cancel.cancelled() => {
                        info!("Docker container lifecycle watcher shutting down");
                        return;
                    }
                    event = event_stream.next() => {
                        match event {
                            Some(Ok(docker_event)) => {
                                handle_docker_event(
                                    &docker_event,
                                    &event_publisher,
                                    max_crash_restart_count,
                                    &client,
                                ).await;
                            }
                            Some(Err(e)) => {
                                warn!(error = %e, "Docker events stream error, reconnecting in 5s");
                                break; // Break inner loop to reconnect
                            }
                            None => {
                                warn!("Docker events stream ended, reconnecting in 5s");
                                break; // Break inner loop to reconnect
                            }
                        }
                    }
                }
            }

            // Wait before reconnecting
            tokio::select! {
                () = cancel.cancelled() => {
                    info!("Docker container lifecycle watcher shutting down during reconnect");
                    return;
                }
                () = tokio::time::sleep(std::time::Duration::from_secs(5)) => {}
            }
        }
    })
}

/// Handle a single Docker event and publish the corresponding lifecycle event.
async fn handle_docker_event(
    event: &bollard::models::EventMessage,
    event_publisher: &Arc<dyn EventPublisher>,
    max_crash_restart_count: i32,
    client: &DockerClient,
) {
    let action = event.action.as_deref().unwrap_or("");
    let Some(actor) = &event.actor else {
        return;
    };

    // Extract task_id from container labels
    let task_id = actor
        .attributes
        .as_ref()
        .and_then(|attrs| attrs.get("moatless.io/task-id"))
        .map(String::as_str);

    let Some(task_id) = task_id else {
        debug!(action, "Docker event has no task-id label, skipping");
        return;
    };

    let container_name = actor
        .attributes
        .as_ref()
        .and_then(|attrs| attrs.get("name"))
        .map_or("unknown", String::as_str);

    debug!(
        task_id,
        action,
        container = container_name,
        "Docker container event"
    );

    match action {
        "start" => {
            event_publisher
                .publish_sandbox_event(task_id, moatless_events::SandboxStatus::Ready, "Running")
                .await;
        }
        "die" | "stop" => {
            // Check if this is a crash loop
            if max_crash_restart_count > 0 {
                if let Ok(info) = client.docker.inspect_container(container_name, None).await {
                    let restart_count = info.restart_count.unwrap_or(0);
                    if restart_count as i32 >= max_crash_restart_count {
                        warn!(
                            task_id,
                            restart_count, "Container in crash loop, publishing error event"
                        );
                        event_publisher
                            .publish_sandbox_event(
                                task_id,
                                moatless_events::SandboxStatus::Error,
                                &format!("Crash loop detected (restarts: {restart_count})"),
                            )
                            .await;
                        return;
                    }
                }
            }

            event_publisher
                .publish_sandbox_event(task_id, moatless_events::SandboxStatus::Stopped, "Stopped")
                .await;
        }
        "oom" => {
            event_publisher
                .publish_sandbox_event(task_id, moatless_events::SandboxStatus::Error, "OOM killed")
                .await;
        }
        "destroy" => {
            event_publisher
                .publish_sandbox_event(
                    task_id,
                    moatless_events::SandboxStatus::Removed,
                    "Container removed",
                )
                .await;
        }
        _ => {
            debug!(task_id, action, "Unhandled Docker event action");
        }
    }
}
