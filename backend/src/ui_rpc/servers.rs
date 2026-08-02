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

use crate::sandbox::access::{LogOptions, SandboxIntentReader, SandboxObserver, SandboxStarter};
use crate::sandbox::error::SandboxError;
use crate::sandbox::schemas::ServerEntry;
use crate::task::dao::TaskDao;
use moatless_backend_core::scope::ScopeContext;

/// How often a subscribed status stream asks the runtime what changed.
///
/// A container going ready is the transition this exists to make visible, and
/// two seconds is short enough that it reads as immediate to someone watching.
/// Nothing is emitted unless the projected list actually moved, so an idle
/// subscription costs one status read per interval and no frames.
const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// Everything the `servers.*` handlers read through.
///
/// `ui_rpc` cannot reuse the sandbox module's own `authorize_task_read` and
/// `resolve_desired_state` — both are `pub(super)` on `SandboxApiState` — so the
/// same two reads are performed here against the same two handles.
#[derive(Clone)]
pub struct ServersState {
    pub observer: Arc<dyn SandboxObserver>,
    pub tasks: Arc<dyn TaskDao>,
    pub intent: Arc<dyn SandboxIntentReader>,
    pub starter: Arc<dyn SandboxStarter>,
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
) -> Result<crate::task::models::Task, ServersError> {
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
/// declaration, and `sandboxStatus` is the field that says whether anything is
/// running at all.
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
        "sandboxStatus": status.sandbox_status.as_str(),
    }))
}

/// A Task's sandbox lifecycle status only, without any declared server rows.
pub async fn sandbox_status(
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
        .live_status(task_id, task.status, desired)
        .await
        .map_err(ServersError::Sandbox)?;
    Ok(json!({
        "sandboxStatus": status.sandbox_status.as_str(),
    }))
}

/// Request that a Task's Sandbox move toward Running, then return its lifecycle
/// status only.
pub async fn start_sandbox(
    state: &ServersState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, ServersError> {
    let _task = authorized_task(state, viewer, task_id).await?;
    state
        .starter
        .start(task_id)
        .await
        .map_err(ServersError::Sandbox)?;
    sandbox_status(state, viewer, task_id).await
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
    request_id: crate::ui_rpc::RequestId,
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
                    .send(crate::ui_rpc::failure_exit(
                        &request_id,
                        "Conversation not found",
                    ))
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
    request_id: crate::ui_rpc::RequestId,
    state: ServersState,
    viewer: ScopeContext,
    task_id: String,
    server_name: String,
    previous: bool,
    frames: mpsc::Sender<Value>,
) {
    if let Err(error) = authorized_task(&state, &viewer, &task_id).await {
        let _ = frames
            .send(crate::ui_rpc::failure_exit(
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
                .send(crate::ui_rpc::failure_exit(
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
    use crate::sandbox::models::{ServerCapabilities, ServerStatus};

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

/// The half of a `servers.list` answer the database decides.
///
/// Config-first is a claim about where the list comes from, and where it comes
/// from is SQL: a Task resolves to a Workspace, the Workspace's `servers` JSONB
/// declares them, and the Task's `disabled_servers` removes the ones the
/// crash-loop guard dropped. None of that is visible without a real database,
/// and neither is the scoped read that decides whether this viewer may see the
/// Task at all — which is the same `find_by_id` every other read goes through
/// and the reason an unauthorized caller gets `NotFound` rather than an empty
/// list.
///
/// What the cluster reports for each row is the one thing this tier cannot
/// supply, so the observer here answers the way the resolver does for a Task
/// with no pod, and the assertions are about the list rather than its statuses.
#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::*;
    use crate::sandbox::access::SandboxConnector;
    use crate::sandbox::auto_provision::get_server_configs_for_task;
    use crate::sandbox::client::SandboxClient;
    use crate::sandbox::live_status::LiveStatus;
    use crate::sandbox::models::{SandboxStatus, ServerStatus, server_capabilities_from_config};
    use crate::sandbox::schemas::SandboxStatusFullResponse;
    use crate::task::dao::PgTaskDao;
    use crate::task::models::{SandboxDesiredState, TaskStatus};
    use crate::workspace::dao::PgWorkspaceDao;
    use async_trait::async_trait;
    use futures::stream::BoxStream;
    use sqlx::PgPool;
    use std::sync::Arc;

    /// A Task that never had a sandbox: the servers it declares, read from the
    /// database exactly as the production adapter reads them, and nothing
    /// observed to overlay on them.
    struct NeverProvisioned(PgPool);

    #[async_trait]
    impl SandboxObserver for NeverProvisioned {
        async fn sandbox_status(
            &self,
            task_id: &str,
            _task_status: TaskStatus,
            _desired_state: SandboxDesiredState,
        ) -> Result<SandboxStatusFullResponse, SandboxError> {
            let configs = get_server_configs_for_task(
                &PgTaskDao(self.0.clone()),
                &PgWorkspaceDao(self.0.clone()),
                task_id,
            )
            .await;
            Ok(SandboxStatusFullResponse {
                sandbox_status: SandboxStatus::NotCreated,
                sandbox_error: None,
                agent_status: None,
                servers: configs
                    .iter()
                    .map(|config| ServerEntry {
                        name: config.name.clone(),
                        label: config.label.clone().unwrap_or_else(|| config.name.clone()),
                        port: config.port,
                        status: ServerStatus::Starting,
                        url: None,
                        error: None,
                        detail: None,
                        default: config.default,
                        capabilities: server_capabilities_from_config(config),
                    })
                    .collect(),
                resources: None,
                container_diagnostics: None,
                runtime_events: None,
                scheduled_jobs: Vec::new(),
                background_processes: Vec::new(),
                sandbox_desired_state: None,
                current_sandbox_image: None,
                latest_sandbox_image: None,
                image_up_to_date: None,
            })
        }

        async fn live_status(
            &self,
            _task_id: &str,
            _task_status: TaskStatus,
            _desired_state: SandboxDesiredState,
        ) -> Result<LiveStatus, SandboxError> {
            unreachable!("servers.list does not read live status")
        }

        async fn server_logs(
            &self,
            _task_id: &str,
            _server_name: &str,
            _options: crate::sandbox::access::LogOptions,
        ) -> Result<BoxStream<'static, Result<String, SandboxError>>, SandboxError> {
            unreachable!("servers.list does not read logs")
        }
    }

    /// No sandbox was ever asked for, which is the lazy-absence shape a Task
    /// has before any lifecycle operation touches it.
    struct NeverAsked;

    #[async_trait]
    impl SandboxIntentReader for NeverAsked {
        async fn desired_state(&self, _task_id: &str) -> Result<SandboxDesiredState, SandboxError> {
            Ok(SandboxDesiredState::Stopped)
        }
    }

    #[async_trait]
    impl SandboxConnector for NeverAsked {
        async fn sandbox_client(&self, _task_id: &str) -> Result<SandboxClient, SandboxError> {
            unreachable!("servers.list does not connect to a sandbox")
        }
    }

    #[async_trait]
    impl SandboxStarter for NeverAsked {
        async fn start(&self, _task_id: &str) -> Result<(), SandboxError> {
            unreachable!("servers.list does not start a sandbox")
        }
    }

    fn state(pool: &PgPool) -> ServersState {
        ServersState {
            observer: Arc::new(NeverProvisioned(pool.clone())),
            tasks: Arc::new(PgTaskDao(pool.clone())),
            intent: Arc::new(NeverAsked),
            starter: Arc::new(NeverAsked),
        }
    }

    fn viewer(user_id: &str) -> ScopeContext {
        ScopeContext {
            user_id: user_id.to_string(),
            is_admin: false,
        }
    }

    /// One Workspace declaring three servers, one Task pointing at it with one
    /// of the three auto-disabled.
    async fn seed(pool: &PgPool, task_id: &str, owner: &str) {
        sqlx::query(
            "INSERT INTO teams (id, name, slug) VALUES ('team-x', 'Team X', 'team-x')
             ON CONFLICT (id) DO NOTHING",
        )
        .execute(pool)
        .await
        .expect("seed team");

        sqlx::query(
            r#"INSERT INTO workspaces (id, name, kind, servers, scope, owner_user_id, created_by)
               VALUES ('ws-1', 'ws-1', 'blank', $1, 'user', $2, $2)"#,
        )
        .bind(serde_json::json!([
            { "name": "web", "label": "Web", "port": 5733, "default": true },
            { "name": "docs", "port": 4000 },
            { "name": "flaky", "port": 4100 },
        ]))
        .bind(owner)
        .execute(pool)
        .await
        .expect("seed workspace");

        sqlx::query(
            r#"INSERT INTO tasks (task_id, created_at, last_message_at, agent_type, repository_id,
                   status, total_cost_usd, visibility, scope, team_id, created_by, owner_user_id,
                   workspace_id, disabled_servers)
               VALUES ($1, NOW()::text, NOW()::text, 'claude-code', 'repo-x', 'open', 0,
                   'private', 'user', 'team-x', $2, $2, 'ws-1', $3)"#,
        )
        .bind(task_id)
        .bind(owner)
        .bind(serde_json::json!(["flaky"]))
        .execute(pool)
        .await
        .expect("seed task");
    }

    #[sqlx::test]
    async fn a_task_whose_sandbox_was_never_provisioned_still_lists_what_it_declares(pool: PgPool) {
        seed(&pool, "task-1", "user-1").await;

        let listed = list(&state(&pool), &viewer("user-1"), "task-1")
            .await
            .expect("the owner may read their own Task");

        // The environment does not exist. That is the field that says so — not
        // the per-server statuses, which have nothing behind them to report.
        assert_eq!(listed["sandboxStatus"], "not_created");

        let servers = listed["servers"].as_array().expect("an array of servers");
        assert_eq!(
            servers
                .iter()
                .map(|server| server["name"].as_str().unwrap_or_default())
                .collect::<Vec<_>>(),
            vec!["web", "docs"],
            "the disabled server is not listed, and the rest are"
        );
        // `label` falls back to the name, and the port comes from the JSONB.
        assert_eq!(servers[0]["label"], "Web");
        assert_eq!(servers[0]["port"], 5733);
        assert_eq!(servers[0]["default"], true);
        assert_eq!(servers[1]["label"], "docs");
        assert_eq!(servers[1]["port"], 4000);
        // Required and nullable on the contract, so written rather than skipped.
        assert!(servers[1].get("error").is_some_and(Value::is_null));
        assert!(servers[1].get("detail").is_some_and(Value::is_null));
    }

    /// A Task another tenant cannot read is not an empty list — it is missing.
    /// An empty list would tell the caller the Task exists and declares nothing.
    #[sqlx::test]
    async fn a_viewer_who_cannot_read_the_task_is_told_it_does_not_exist(pool: PgPool) {
        seed(&pool, "task-1", "user-1").await;

        let denied = list(&state(&pool), &viewer("user-2"), "task-1").await;

        assert!(
            matches!(denied, Err(ServersError::NotFound)),
            "expected NotFound, got {denied:?}"
        );
    }
}
