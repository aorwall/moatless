//! Docker container lifecycle watcher.
//!
//! Watches Docker engine events for sandbox containers (identified by managed
//! and sandbox-service labels) and publishes lifecycle events to the event
//! system. Analogous to `k8s/pod_watcher.rs` but for Docker mode.

use std::collections::HashMap;
use std::sync::Arc;

use bollard::query_parameters::EventsOptionsBuilder;
use futures::StreamExt;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::client::DockerClient;
use super::container_builder::{
    CREATED_BY_LABEL, CREATED_BY_VALUE, SANDBOX_SERVICE, SERVICE_LABEL,
};
use crate::events::EventPublisher;
use crate::sandbox::lifecycle::classify::ObservedTransition;
use crate::sandbox::lifecycle::runtime::{generation_from_label, OPERATION_GENERATION_LABEL};
use crate::sandbox::lifecycle::watcher::{classify_observation, publish_sandbox_observation};
use crate::sandbox::lifecycle::Lifecycle;

/// Spawn a background task that watches Docker container events and publishes
/// lifecycle events to the event system.
///
/// Subordinate server-container events are excluded by the service-label
/// filter and a defensive handler check, so they never enter classification.
pub fn spawn_container_lifecycle_watcher(
    client: DockerClient,
    event_publisher: Arc<dyn EventPublisher>,
    cancel: CancellationToken,
    max_crash_restart_count: i32,
    lifecycle: Option<Arc<Lifecycle>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!("Docker container lifecycle watcher started");

        loop {
            // Only the sandbox container participates in Sandbox lifecycle.
            let filters = event_filters();

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
                                    lifecycle.as_ref(),
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
    lifecycle: Option<&Arc<Lifecycle>>,
) {
    let Some(event) = sandbox_event(event) else {
        debug!("Ignoring non-sandbox Docker container event");
        return;
    };
    let action = event.action;
    let attributes = event.attributes;

    // Extract task_id from container labels
    let task_id = attributes.get("moatless.io/task-id").map(String::as_str);

    let Some(task_id) = task_id else {
        debug!(action, "Docker event has no task-id label, skipping");
        return;
    };

    let container_name = attributes.get("name").map_or("unknown", String::as_str);
    let operation_generation = generation_from_label(attributes.get(OPERATION_GENERATION_LABEL));

    debug!(
        task_id,
        action,
        container = container_name,
        "Docker container event"
    );

    match action {
        "start" => {
            let decision = classify_observation(
                lifecycle,
                task_id,
                ObservedTransition::Ready,
                operation_generation,
            )
            .await;
            if decision.publishes {
                let payload = moatless_events::SandboxEventPayload {
                    message: "Running".to_string(),
                    ..Default::default()
                };
                publish_sandbox_observation(
                    event_publisher.as_ref(),
                    task_id,
                    moatless_events::SandboxStatus::Ready,
                    &payload,
                    decision.event_generation,
                )
                .await;
            }
        }
        "die" | "stop" => {
            let mut transition = ObservedTransition::CleanStop;
            match client.docker.inspect_container(container_name, None).await {
                Ok(info) => {
                    let restart_count = info.restart_count.unwrap_or(0);
                    transition = stopped_transition(&info);
                    if max_crash_restart_count > 0
                        && restart_count as i32 >= max_crash_restart_count
                    {
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
                Err(e) => {
                    warn!(
                        task_id,
                        container = container_name,
                        error = %e,
                        "Failed to inspect stopped Docker sandbox; treating as clean stop"
                    );
                }
            }

            if classify_observation(lifecycle, task_id, transition, operation_generation)
                .await
                .publishes
            {
                let (status, message) = match transition {
                    ObservedTransition::Crash | ObservedTransition::NonZeroStop => {
                        (moatless_events::SandboxStatus::Error, "Container crashed")
                    }
                    _ => (moatless_events::SandboxStatus::Stopped, "Stopped"),
                };
                event_publisher
                    .publish_sandbox_event(task_id, status, message)
                    .await;
            }
        }
        "oom" => {
            if classify_observation(
                lifecycle,
                task_id,
                ObservedTransition::OomKilled,
                operation_generation,
            )
            .await
            .publishes
            {
                event_publisher
                    .publish_sandbox_event(
                        task_id,
                        moatless_events::SandboxStatus::Error,
                        "OOM killed",
                    )
                    .await;
            }
        }
        "destroy" => {
            if classify_observation(
                lifecycle,
                task_id,
                ObservedTransition::Deleted,
                operation_generation,
            )
            .await
            .publishes
            {
                event_publisher
                    .publish_sandbox_event(
                        task_id,
                        moatless_events::SandboxStatus::Removed,
                        "Container removed",
                    )
                    .await;
            }
        }
        _ => {
            debug!(task_id, action, "Unhandled Docker event action");
        }
    }
}

fn stopped_transition(info: &bollard::models::ContainerInspectResponse) -> ObservedTransition {
    if info
        .state
        .as_ref()
        .and_then(|state| state.exit_code)
        .is_some_and(|code| code != 0)
    {
        ObservedTransition::NonZeroStop
    } else {
        ObservedTransition::CleanStop
    }
}

fn event_filters() -> HashMap<String, Vec<String>> {
    HashMap::from([
        ("type".to_string(), vec!["container".to_string()]),
        (
            "label".to_string(),
            vec![
                format!("{CREATED_BY_LABEL}={CREATED_BY_VALUE}"),
                format!("{SERVICE_LABEL}={SANDBOX_SERVICE}"),
            ],
        ),
        (
            "event".to_string(),
            vec![
                "start".to_string(),
                "die".to_string(),
                "stop".to_string(),
                "destroy".to_string(),
                "oom".to_string(),
            ],
        ),
    ])
}

struct SandboxEvent<'a> {
    action: &'a str,
    attributes: &'a HashMap<String, String>,
}

fn sandbox_event(event: &bollard::models::EventMessage) -> Option<SandboxEvent<'_>> {
    let action = event.action.as_deref().unwrap_or("");
    let attributes = event.actor.as_ref()?.attributes.as_ref()?;
    (attributes.get(SERVICE_LABEL).map(String::as_str) == Some(SANDBOX_SERVICE))
        .then_some(SandboxEvent { action, attributes })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_labeled_event_is_rejected_before_classification() {
        let event = bollard::models::EventMessage {
            action: Some("die".to_string()),
            actor: Some(bollard::models::EventActor {
                attributes: Some(HashMap::from([
                    (SERVICE_LABEL.to_string(), "server".to_string()),
                    ("moatless.io/task-id".to_string(), "t1".to_string()),
                ])),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert!(sandbox_event(&event).is_none());
    }

    #[test]
    fn nonzero_exit_remains_correlatable_until_classification() {
        let inspected = bollard::models::ContainerInspectResponse {
            state: Some(bollard::models::ContainerState {
                exit_code: Some(137),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            stopped_transition(&inspected),
            ObservedTransition::NonZeroStop
        );
    }
}
