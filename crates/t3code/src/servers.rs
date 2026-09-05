//! What servers a Task declares, what state each is in, and what one is printing.
//!
//! Everything here is a read. Nothing in this module starts, stops, restarts or
//! reconfigures a server — those are specified in `.plans/moatless-convergence.md`
//! and deliberately out of scope for the change this module belongs to.
//!
//! The data is not new: [`SandboxObserver::sandbox_status`] already assembles it
//! for the Moatless UI's own status route, config-first, straight from the K8s
//! API with no cache in between. This module projects that answer onto T3's
//! `servers.*` contract and streams it, so a T3 client learns about a server the
//! same way it learns about everything else — over the one socket it already has.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tracing::warn;

use crate::task_dao::{T3TaskDao, TaskView};
use moatless_ports::sandbox::{
    LogOptions, SandboxError, SandboxIntentReader, SandboxObserver, ServerEntry,
};
use moatless_types::access_control::ScopeContext;

/// How often a subscribed status stream asks the runtime what changed.
///
/// A container going ready is the transition this exists to make visible, and
/// two seconds is short enough that it reads as immediate to someone watching.
/// Nothing is emitted unless the projected list actually moved, so an idle
/// subscription costs one status read per interval and no frames.
const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Everything the `servers.*` handlers read through.
///
/// `t3code` cannot reuse the sandbox module's own `authorize_task_read` and
/// `resolve_desired_state` — both belong to the root backend — so the
/// same two reads are performed here against the same two handles.
#[derive(Clone)]
pub struct ServersState {
    pub observer: Arc<dyn SandboxObserver>,
    pub tasks: Arc<dyn T3TaskDao>,
    pub intent: Arc<dyn SandboxIntentReader>,
}

/// Why a `servers.*` read could not be answered.
#[derive(Debug, thiserror::Error)]
pub enum ServersError {
    /// The viewer cannot read this Task, or it does not exist. The two are the
    /// same answer on purpose: a scoped read that returns nothing must not let
    /// a caller distinguish another tenant's Task from a missing one.
    #[error("conversation not found")]
    NotFound,
    #[error("the sandbox could not be read")]
    Sandbox(#[source] SandboxError),
    #[error("the task could not be read")]
    Database(#[source] sqlx::Error),
}

/// One server, in the shape `ThreadServer` has on the wire
/// (`packages/contracts/src/servers.ts`).
///
/// Field-for-field from [`ServerEntry`], which is why this is a projection and
/// not a translation: the contract was written from the Rust shape so that a
/// reviewer comparing the two is reading the same names twice.
fn project_server(entry: &ServerEntry) -> Value {
    json!({
        "name": entry.name,
        "label": entry.label,
        "port": entry.port,
        "status": entry.status.to_string(),
        "url": entry.url,
        // `error` and `detail` are skipped when absent on the HTTP response,
        // where a missing key and a null mean the same thing to its consumer.
        // The contract here declares them nullable and required, so they are
        // always written — a client that decodes a struct does not get to
        // treat an absent field as null.
        "error": entry.error,
        "detail": entry.detail,
        "default": entry.default,
    })
}

/// Resolve the Task for a viewer, failing closed when there is no identity.
async fn authorized_task(
    state: &ServersState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<TaskView, ServersError> {
    state
        .tasks
        .find_by_id(task_id, viewer)
        .await
        .map_err(ServersError::Database)?
        .ok_or(ServersError::NotFound)
}

/// Every server declared by a Task, with the state the runtime reports for it.
///
/// A Task whose sandbox was never provisioned still answers: status resolution
/// is config-first, so the servers its Workspace declares are listed rather
/// than the list being empty. What each row *says* in that case is the
/// resolver's fallback and not an observation — `starting`, with the ingress
/// URL its port will have — so a client showing one of these rows is showing a
/// server declaration only. Sandbox lifecycle status is exposed by
/// `sandbox.status`.
pub async fn list(
    state: &ServersState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, ServersError> {
    let task = authorized_task(state, viewer, task_id).await?;
    let desired = state
        .intent
        .desired_state(task_id)
        .await
        .map_err(ServersError::Sandbox)?;
    let status = state
        .observer
        .sandbox_status(task_id, task.status, desired)
        .await
        .map_err(ServersError::Sandbox)?;
    Ok(json!({
        "servers": status.servers.iter().map(project_server).collect::<Vec<_>>(),
    }))
}

/// One snapshot of a Task's servers, for the status stream.
async fn status_snapshot(
    state: &ServersState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, ServersError> {
    let servers = list(state, viewer, task_id).await?;
    Ok(json!({
        "threadId": task_id,
        "servers": servers.get("servers").cloned().unwrap_or(Value::Array(vec![])),
        "observedAt": Utc::now().to_rfc3339(),
    }))
}

/// Push a Task's server list for as long as the subscription lives.
///
/// Polling rather than watching, and that is a decision rather than a shortcut.
/// A push path exists — the pod watcher publishes convergence events to Redis
/// Streams — but what it classifies is the *sandbox* converging, not a server
/// container's probe flipping, which is the transition this stream is about.
///
/// A poll that produces the same list as the last one emitted sends nothing, so
/// the interval is the coalescing window and an idle subscription is silent. A
/// failed poll leaves the last emitted list where it was and the subscription
/// survives: ending it would present a transient K8s error as a dead panel, and
/// the next poll covers the same ground.
pub async fn watch_status(
    request_id: crate::RequestId,
    state: ServersState,
    viewer: ScopeContext,
    task_id: String,
    frames: mpsc::Sender<Value>,
) {
    let mut last: Option<Value> = None;
    loop {
        tokio::time::sleep(STATUS_POLL_INTERVAL).await;
        let snapshot = match status_snapshot(&state, &viewer, &task_id).await {
            Ok(snapshot) => snapshot,
            Err(ServersError::NotFound) => {
                // The conversation stopped being readable. No poll can recover
                // from that, and a silent stream would leave the panel showing
                // its last list as current.
                let _ = frames
                    .send(crate::failure_exit(&request_id, "Conversation not found"))
                    .await;
                return;
            }
            Err(error) => {
                warn!(%error, task_id, "Failed to read a Task's servers");
                continue;
            }
        };
        // `observedAt` moves on every poll, so the comparison is on the list.
        let servers = snapshot.get("servers").cloned();
        if last == servers {
            continue;
        }
        last = servers;
        let chunk = json!({
            "_tag": "Chunk",
            "requestId": request_id.wire(),
            "values": [snapshot],
        });
        if frames.send(chunk).await.is_err() {
            return;
        }
    }
}

/// Send one server's log lines for as long as the subscription lives.
///
/// The stream ends when the container's does. Nothing is retained, so a client
/// that reconnects receives only new lines and the gap is not backfilled —
/// which is a fact the panel states rather than hides.
pub async fn watch_logs(
    request_id: crate::RequestId,
    state: ServersState,
    viewer: ScopeContext,
    task_id: String,
    server_name: String,
    previous: bool,
    frames: mpsc::Sender<Value>,
) {
    if let Err(error) = authorized_task(&state, &viewer, &task_id).await {
        let _ = frames
            .send(crate::failure_exit(
                &request_id,
                match error {
                    ServersError::NotFound => "Conversation not found".to_string(),
                    error => format!("Failed to read the Task: {error}"),
                },
            ))
            .await;
        return;
    }

    // A subscription is open-ended by definition, so it follows. `previous`
    // asks for the instance that died, which cannot be followed — the option
    // struct already says so, and this passes the caller's choice through
    // rather than restating the rule.
    let options = LogOptions {
        previous,
        follow: !previous,
        ..LogOptions::default()
    };
    let mut lines = match state
        .observer
        .server_logs(&task_id, &server_name, options)
        .await
    {
        Ok(lines) => lines,
        Err(error) => {
            // "Not ready" and "no previous logs" are ordinary answers a
            // reviewer needs to see, so the reason reaches the client rather
            // than only the log.
            let _ = frames
                .send(crate::failure_exit(
                    &request_id,
                    format!("Server logs unavailable: {error}"),
                ))
                .await;
            return;
        }
    };

    while let Some(line) = lines.next().await {
        let line = match line {
            Ok(line) => line,
            Err(error) => {
                warn!(%error, task_id, server_name, "A server log stream failed");
                return;
            }
        };
        let chunk = json!({
            "_tag": "Chunk",
            "requestId": request_id.wire(),
            "values": [{
                "threadId": task_id,
                "name": server_name,
                "line": line,
            }],
        });
        if frames.send(chunk).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_ports::sandbox::{ServerCapabilities, ServerStatus};

    fn entry(status: ServerStatus) -> ServerEntry {
        ServerEntry {
            name: "web".to_string(),
            label: "Web".to_string(),
            port: 5733,
            status,
            url: Some("https://task--5733.example.com".to_string()),
            error: None,
            detail: None,
            default: true,
            capabilities: ServerCapabilities {
                address_bar: true,
                inspector: false,
                reload: true,
                open_external: true,
                viewport: true,
                console: false,
            },
        }
    }

    #[test]
    fn a_started_server_projects_field_for_field() {
        assert_eq!(
            project_server(&entry(ServerStatus::Started)),
            json!({
                "name": "web",
                "label": "Web",
                "port": 5733,
                "status": "started",
                "url": "https://task--5733.example.com",
                "error": null,
                "detail": null,
                "default": true
            })
        );
    }

    /// A server with no URL is the ordinary case before its pod exists, and the
    /// contract declares `url` nullable so the row can still be listed.
    #[test]
    fn a_server_with_no_url_projects_a_null_rather_than_being_dropped() {
        let mut without_url = entry(ServerStatus::Stopped);
        without_url.url = None;
        let projected = project_server(&without_url);
        assert_eq!(projected["url"], Value::Null);
        assert_eq!(projected["status"], "stopped");
    }

    /// `error` and `detail` are skipped on the HTTP response when absent. The
    /// contract requires them, so the projection has to write them anyway —
    /// this is the test that fails if someone reuses the HTTP serialization.
    #[test]
    fn a_failed_server_carries_its_error_and_detail() {
        let mut failed = entry(ServerStatus::Failed);
        failed.error = Some("CrashLoopBackOff".to_string());
        failed.detail = Some("exit code 127".to_string());
        let projected = project_server(&failed);
        assert_eq!(projected["status"], "failed");
        assert_eq!(projected["error"], "CrashLoopBackOff");
        assert_eq!(projected["detail"], "exit code 127");
    }

    /// Every value of the status enum has to be one the contract's literal
    /// union accepts, or a client silently drops the row it cannot decode.
    #[test]
    fn every_status_projects_to_a_value_the_contract_declares() {
        let declared = ["stopped", "installing", "starting", "started", "failed"];
        for status in [
            ServerStatus::Stopped,
            ServerStatus::Installing,
            ServerStatus::Starting,
            ServerStatus::Started,
            ServerStatus::Failed,
        ] {
            let projected = project_server(&entry(status));
            let projected = projected["status"].as_str().unwrap().to_string();
            assert!(
                declared.contains(&projected.as_str()),
                "{projected} is not in the contract's ServerRuntimeStatus"
            );
        }
    }
}
