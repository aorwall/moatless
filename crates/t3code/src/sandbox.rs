//! Sandbox lifecycle status and commands for the T3 compatibility surface.
//!
//! This module owns only environment lifecycle answers: whether a Task's
//! Sandbox is available, and start/stop requests. Preview server declarations
//! and server logs live in `servers.rs`.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tracing::warn;

use crate::subscription::{StreamFailure, StreamResult, StreamSink};
use crate::task_dao::{T3TaskDao, TaskView};
use moatless_ports::sandbox::{
    ClusterDiagnostics, SandboxError, SandboxIntentReader, SandboxObserver, SandboxOperator,
    SandboxStarter, SandboxStopper,
};
use moatless_types::access_control::ScopeContext;
use moatless_types::sandbox::SandboxStopReason;

/// How often a `sandbox.subscribeStatus` subscriber re-reads lifecycle state.
/// Matched to the server-list poller so the two panels move together.
const STATUS_POLL_INTERVAL: Duration = Duration::from_secs(2);

/// How often a `sandbox.subscribeDetail` subscriber re-reads the panel's data.
/// Five times the status interval: this read reaches the runtime's diagnostics
/// and metrics APIs rather than its lifecycle state, and the panel it feeds is
/// opened deliberately rather than always shown.
const DETAIL_POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Everything the sandbox lifecycle handlers need.
#[derive(Clone)]
pub struct SandboxState {
    pub observer: Arc<dyn SandboxObserver>,
    pub tasks: Arc<dyn T3TaskDao>,
    pub intent: Arc<dyn SandboxIntentReader>,
    pub starter: Arc<dyn SandboxStarter>,
    pub stopper: Arc<dyn SandboxStopper>,
    /// Restart, redeploy and cleanup, for the sandbox panel's other controls.
    pub operator: Arc<dyn SandboxOperator>,
    /// Fleet-wide status and live resource usage. Deployment-scoped rather than
    /// per-Task, so it takes no `tasks` authorization: the caller's environment
    /// scope is the whole of it.
    pub diagnostics: Arc<dyn ClusterDiagnostics>,
}

/// Why a sandbox lifecycle request could not be answered.
#[derive(Debug, thiserror::Error)]
pub enum SandboxRpcError {
    /// The viewer cannot read this Task, or it does not exist.
    #[error("conversation not found")]
    NotFound,
    #[error("the sandbox could not be read")]
    Sandbox(#[source] SandboxError),
    #[error("the task could not be read")]
    Database(#[source] sqlx::Error),
    /// Moatless declined the change itself, after the gate here allowed it.
    #[error("{0}")]
    Refused(String),
}

/// The Task, if this viewer may be told it exists. The read gate, for the
/// status calls.
async fn readable_task(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<TaskView, SandboxRpcError> {
    state
        .tasks
        .find_by_id(task_id, viewer)
        .await
        .map_err(SandboxRpcError::Database)?
        .ok_or(SandboxRpcError::NotFound)
}

/// The Task, if this viewer may operate it. The gate on start and stop, which
/// move a Sandbox rather than report on one.
async fn controlled_task(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<TaskView, SandboxRpcError> {
    state
        .tasks
        .controlled_by_id(task_id, viewer)
        .await
        .map_err(SandboxRpcError::Database)?
        .ok_or(SandboxRpcError::NotFound)
}

/// The Task, if this viewer may write it. The gate on cleanup, which deletes a
/// Sandbox and its archives rather than moving one between states.
async fn written_task(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<TaskView, SandboxRpcError> {
    state
        .tasks
        .written_by_id(task_id, viewer)
        .await
        .map_err(SandboxRpcError::Database)?
        .ok_or(SandboxRpcError::NotFound)
}

/// A Task's sandbox lifecycle status only, without declared server rows.
pub async fn status(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, SandboxRpcError> {
    let task = readable_task(state, viewer, task_id).await?;
    let desired = state
        .intent
        .desired_state(task_id)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    let status = state
        .observer
        .live_status(task_id, task.status, desired)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    let mut payload = json!({
        "sandboxStatus": status.sandbox_status.as_str(),
    });
    // Absent, not null, when the runtime has no answer — a sandbox that is not
    // `Ready` has no agent to report on, and a `null` fails the client's decode
    // of the whole result. The key is `optionalKey` in the contract for the
    // same reason, and a client that has not read it is unaffected.
    if let Some(agent) = status.agent_status {
        payload["agentStatus"] = json!(agent.as_str());
    }
    // Why the sandbox is where it is, when the runtime said. Absent for the
    // same reason `agentStatus` is, and the sandbox panel's Status section is
    // the only thing that renders it.
    if let Some(reason) = status.sandbox_error {
        payload["sandboxError"] = json!(reason);
    }
    // Omitted rather than sent empty, for the same reason: a client that has
    // not read the key sees the payload it always saw, and one that has can
    // tell "no commands" from a server that does not report them by reading
    // `capabilities.sandboxCommands` rather than by guessing from an empty
    // array.
    if !status.commands.is_empty() {
        payload["commands"] = json!(status.commands);
    }
    Ok(payload)
}

/// Everything the sandbox panel shows beside the lifecycle state: the image, the
/// containers and their metrics, the runtime events, and the idle-stop TTL.
///
/// Read gate rather than control gate: this reports on a Sandbox and moves
/// nothing, and the panel is open for anyone who can see the thread.
pub async fn detail(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, SandboxRpcError> {
    let task = readable_task(state, viewer, task_id).await?;
    let desired = state
        .intent
        .desired_state(task_id)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    let detail = state
        .observer
        .detail(task_id, task.status, desired)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    let mut payload = json!({
        "desiredState": desired.as_str(),
        "idleTimeoutMinutes": task.ttl_minutes,
    });
    // Each section is absent rather than null or empty when the runtime has no
    // answer for it, so the panel hides the section instead of rendering zeroes
    // a docker-compose deployment would never fill in. `status` states the rule.
    if let Some(image) = image(state, task_id, detail.current_image).await {
        payload["image"] = image;
    }
    if !detail.containers.is_empty() {
        payload["containers"] = Value::Array(
            detail
                .containers
                .into_iter()
                .map(|container| {
                    json!({
                        "name": container.name,
                        "state": container.state,
                        "restartCount": container.restart_count,
                        "exitCode": container.exit_code,
                        "terminatedReason": container.terminated_reason,
                        "lastExitCode": container.last_exit_code,
                        "lastTerminatedReason": container.last_terminated_reason,
                        "waitingReason": container.waiting_reason,
                        "cpuMillicores": container.cpu_millicores,
                        "memoryMb": container.memory_mb,
                    })
                })
                .collect(),
        );
    }
    if !detail.runtime_events.is_empty() {
        payload["runtimeEvents"] = Value::Array(
            detail
                .runtime_events
                .into_iter()
                .map(|event| {
                    json!({
                        "reason": event.reason,
                        "message": event.message,
                        "severity": event.severity,
                        "count": event.count,
                        "lastTimestamp": event.last_timestamp,
                    })
                })
                .collect(),
        );
    }
    Ok(payload)
}

/// The panel's Image section: what the sandbox runs, what a redeploy would give
/// it, and whether those differ. `None` when neither is known, so the panel
/// hides the section rather than showing two blanks.
///
/// The two halves come from different places on purpose. `current` is the pod's,
/// and only the status reader can see it; `latest` is the deployment config's,
/// and only the lifecycle orchestrator resolves it. An unresolvable `latest`
/// degrades to absence rather than to a failure, matching `sandbox_status_handler`
/// — a panel that shows the running image is better than one that shows nothing.
async fn image(state: &SandboxState, task_id: &str, current: Option<String>) -> Option<Value> {
    let latest = match state.operator.expected_image(task_id).await {
        Ok(latest) => Some(latest),
        Err(error) => {
            warn!(%error, task_id, "Failed to resolve the expected sandbox image");
            None
        }
    };
    if current.is_none() && latest.is_none() {
        return None;
    }
    let up_to_date = current
        .as_ref()
        .zip(latest.as_ref())
        .map(|(current, latest)| current == latest);
    Some(json!({
        "current": current,
        "latest": latest,
        "upToDate": up_to_date,
    }))
}

/// What a sandbox subscription re-reads on each tick.
///
/// The two differ in width and in rate, and in nothing else, so one loop serves
/// both and a change-detection fix cannot land on one and miss the other.
#[derive(Debug, Clone, Copy)]
enum Watched {
    /// `sandbox.subscribeStatus`: the lifecycle state behind the indicator.
    Status,
    /// `sandbox.subscribeDetail`: the sandbox panel's wider read.
    Detail,
}

impl Watched {
    /// The tag the subscription opened under, for the refusal that closes it.
    fn tag(self) -> &'static str {
        match self {
            Self::Status => "sandbox.subscribeStatus",
            Self::Detail => "sandbox.subscribeDetail",
        }
    }

    fn interval(self) -> Duration {
        match self {
            Self::Status => STATUS_POLL_INTERVAL,
            Self::Detail => DETAIL_POLL_INTERVAL,
        }
    }

    async fn read(
        self,
        state: &SandboxState,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<Value, SandboxRpcError> {
        match self {
            Self::Status => status(state, viewer, task_id).await,
            Self::Detail => detail(state, viewer, task_id).await,
        }
    }
}

/// Push a Task's sandbox status for as long as the subscription lives.
///
/// Lifecycle has no event to hang this on — a Sandbox reaches Ready because a
/// pod did, not because anything here was told — so the push is a poll the
/// server runs once per subscriber instead of every client running its own.
/// Only changes are sent: an idle subscription is silent, so a client reads the
/// absence of a message as "nothing moved" rather than "nothing is known".
///
/// `seeded` is the status the subscribe arm already sent. It starts the change
/// detector, because a subscriber that has been told "ready" and is then told
/// "ready" again a moment later has been sent a change that did not happen.
pub async fn watch_status(
    sink: StreamSink,
    state: SandboxState,
    viewer: ScopeContext,
    task_id: String,
    seeded: Value,
) -> StreamResult {
    watch(Watched::Status, sink, state, viewer, task_id, seeded).await
}

/// The same push for the sandbox panel's wider read; see [`watch_status`].
pub async fn watch_detail(
    sink: StreamSink,
    state: SandboxState,
    viewer: ScopeContext,
    task_id: String,
    seeded: Value,
) -> StreamResult {
    watch(Watched::Detail, sink, state, viewer, task_id, seeded).await
}

async fn watch(
    watched: Watched,
    sink: StreamSink,
    state: SandboxState,
    viewer: ScopeContext,
    task_id: String,
    seeded: Value,
) -> StreamResult {
    let mut last = Some(seeded);
    loop {
        tokio::time::sleep(watched.interval()).await;
        let snapshot = match watched.read(&state, &viewer, &task_id).await {
            Ok(snapshot) => snapshot,
            Err(SandboxRpcError::NotFound) => {
                // The conversation stopped being readable. No poll recovers
                // from that, and staying silent would leave the indicator
                // showing its last answer as current.
                warn!(
                    task_id,
                    tag = watched.tag(),
                    "A T3 UI sandbox subscription's thread stopped being readable"
                );
                return Err(StreamFailure::frame(crate::sandbox_stream_refusal(
                    sink.request_id(),
                    watched.tag(),
                )));
            }
            Err(error) => {
                warn!(%error, task_id, tag = watched.tag(), "Failed to read a Task's sandbox");
                continue;
            }
        };
        if last.as_ref() == Some(&snapshot) {
            continue;
        }
        last = Some(snapshot.clone());
        if !sink.send(snapshot).await {
            return Ok(());
        }
    }
}

/// Request that a Task's Sandbox move toward Running, then return lifecycle
/// status only.
pub async fn start(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, SandboxRpcError> {
    let _task = controlled_task(state, viewer, task_id).await?;
    state
        .starter
        .start(task_id)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    status(state, viewer, task_id).await
}

/// Every Sandbox in the deployment, with its status and live resource usage.
///
/// Deployment-global rather than per-Task: there is no `threadId` to authorize
/// against, so reaching this at all is the authorization, and the read is
/// already scoped to the caller's environment by the RPC dispatch.
pub async fn list_diagnostics(state: &SandboxState) -> Result<Value, SandboxRpcError> {
    let entries = state
        .diagnostics
        .list_sandbox_diagnostics()
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    Ok(json!({ "sandboxes": entries }))
}

/// Request that a Task's Sandbox move toward Stopped, then return lifecycle
/// status only.
pub async fn stop(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, SandboxRpcError> {
    let _task = controlled_task(state, viewer, task_id).await?;
    state
        .stopper
        .stop(task_id, SandboxStopReason::User)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    status(state, viewer, task_id).await
}

/// Replace a Task's Sandbox pod, keeping its deployment and its workspace, then
/// return lifecycle status only.
pub async fn restart(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, SandboxRpcError> {
    let _task = controlled_task(state, viewer, task_id).await?;
    state
        .operator
        .restart(task_id)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    status(state, viewer, task_id).await
}

/// Rebuild a Task's Sandbox deployment from current repository config, keeping
/// its workspace, then return lifecycle status only.
pub async fn redeploy(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, SandboxRpcError> {
    let _task = controlled_task(state, viewer, task_id).await?;
    state
        .operator
        .redeploy(task_id)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    status(state, viewer, task_id).await
}

/// Remove a Task's Sandbox and delete its archives, then return lifecycle
/// status only.
///
/// Gated on write and not on control, matching the REST `/cleanup` route: this
/// is a change the owner would have to undo, so `visibility = 'public'` does not
/// widen it.
pub async fn cleanup(
    state: &SandboxState,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Value, SandboxRpcError> {
    let _task = written_task(state, viewer, task_id).await?;
    state
        .operator
        .cleanup(task_id)
        .await
        .map_err(SandboxRpcError::Sandbox)?;
    status(state, viewer, task_id).await
}

/// Set how long a Task's Sandbox may sit idle before the reaper stops it.
///
/// Written through [`ThreadCommandSink`] rather than through a sandbox port: the
/// value lives on the Task, and `tasks` is not this crate's to write. `0`
/// disables the idle stop.
pub async fn set_idle_timeout(
    commands: &dyn crate::ports::ThreadCommandSink,
    viewer: &ScopeContext,
    task_id: &str,
    minutes: i32,
) -> Result<Value, SandboxRpcError> {
    if minutes < 0 {
        return Err(SandboxRpcError::Refused(
            "An idle timeout cannot be negative.".to_string(),
        ));
    }
    commands
        .set_idle_timeout(viewer, task_id, minutes)
        .await
        .map_err(|refusal| SandboxRpcError::Refused(refusal.message()))?;
    Ok(json!({ "idleTimeoutMinutes": minutes }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicBool, Ordering};

    use async_trait::async_trait;
    use moatless_ports::sandbox::{
        ContractSandboxStatus, LiveStatus, LogOptions, SandboxContainer, SandboxDesiredState,
        SandboxDetail, SandboxStatus, ServerLogStream,
    };
    use moatless_types::sandbox::{CommandSummary, ManagedCommandState};
    use moatless_types::task::TaskStatus;
    use tokio::sync::mpsc;

    use crate::task_dao::{MockT3TaskDao, TaskView};

    /// Reports `Initializing` until `moved` is set, then `Ready`. Once `Ready`,
    /// it also reports the agent and the commands it is asked to report.
    struct MovingStatus {
        moved: AtomicBool,
        agent: Option<moatless_ports::sandbox::AgentStatus>,
        commands: Mutex<Vec<CommandSummary>>,
    }

    fn running(label: &str) -> CommandSummary {
        CommandSummary {
            id: format!("cmd-{label}"),
            label: label.to_string(),
            state: ManagedCommandState::Running,
            started_at_unix_ms: 1_754_000_000_000,
            deadline_unix_ms: 1_754_003_600_000,
            exit_code: None,
        }
    }

    impl MovingStatus {
        fn at(moved: bool) -> Self {
            Self {
                moved: AtomicBool::new(moved),
                agent: None,
                commands: Mutex::new(Vec::new()),
            }
        }

        fn reporting(agent: moatless_ports::sandbox::AgentStatus) -> Self {
            Self {
                moved: AtomicBool::new(true),
                agent: Some(agent),
                commands: Mutex::new(Vec::new()),
            }
        }

        /// Ready, agent idle, with `labels` registered and running — the state
        /// a turn leaves behind when it hands work to `moat cmd`.
        fn running_commands(labels: &[&str]) -> Self {
            Self {
                moved: AtomicBool::new(true),
                agent: Some(moatless_ports::sandbox::AgentStatus::Idle),
                commands: Mutex::new(labels.iter().map(|label| running(label)).collect()),
            }
        }

        fn register(&self, label: &str) {
            self.commands
                .lock()
                .expect("no poisoned lock")
                .push(running(label));
        }
    }

    #[async_trait]
    impl SandboxIntentReader for MovingStatus {
        async fn desired_state(&self, _task_id: &str) -> Result<SandboxDesiredState, SandboxError> {
            Ok(SandboxDesiredState::Running)
        }
    }

    #[async_trait]
    impl SandboxObserver for MovingStatus {
        async fn sandbox_status(
            &self,
            _task_id: &str,
            _task_status: TaskStatus,
            _desired_state: SandboxDesiredState,
        ) -> Result<ContractSandboxStatus, SandboxError> {
            unreachable!("the lifecycle read uses live_status")
        }

        async fn live_status(
            &self,
            _task_id: &str,
            _task_status: TaskStatus,
            _desired_state: SandboxDesiredState,
        ) -> Result<LiveStatus, SandboxError> {
            Ok(LiveStatus {
                sandbox_status: if self.moved.load(Ordering::SeqCst) {
                    SandboxStatus::Ready
                } else {
                    SandboxStatus::Initializing
                },
                agent_status: self.agent,
                sandbox_error: None,
                commands: self.commands.lock().expect("no poisoned lock").clone(),
            })
        }

        async fn detail(
            &self,
            _task_id: &str,
            _task_status: TaskStatus,
            _desired_state: SandboxDesiredState,
        ) -> Result<SandboxDetail, SandboxError> {
            let up = self.moved.load(Ordering::SeqCst);
            let containers = if up {
                vec![SandboxContainer {
                    name: "sandbox".to_string(),
                    state: "running".to_string(),
                    restart_count: 0,
                    exit_code: None,
                    terminated_reason: None,
                    last_exit_code: None,
                    last_terminated_reason: None,
                    waiting_reason: None,
                    cpu_millicores: Some(35),
                    memory_mb: Some(512),
                }]
            } else {
                Vec::new()
            };
            Ok(SandboxDetail {
                current_image: up.then(|| "moatless/sandbox:1".to_string()),
                containers,
                runtime_events: Vec::new(),
            })
        }

        async fn server_logs(
            &self,
            _task_id: &str,
            _server_name: &str,
            _options: LogOptions,
        ) -> Result<ServerLogStream, SandboxError> {
            unreachable!("the lifecycle read reads no logs")
        }
    }

    fn watched_state(observer: Arc<MovingStatus>) -> SandboxState {
        let mut tasks = MockT3TaskDao::new();
        tasks.expect_find_by_id().returning(|_, _| {
            Ok(Some(TaskView {
                status: TaskStatus::Open,
                ttl_minutes: 15,
            }))
        });
        SandboxState {
            observer: observer.clone(),
            tasks: Arc::new(tasks),
            intent: observer,
            starter: Arc::new(UnreachableStarter),
            stopper: Arc::new(UnreachableStarter),
            operator: Arc::new(UnreachableStarter),
            diagnostics: Arc::new(UnreachableStarter),
        }
    }

    struct UnreachableStarter;

    #[async_trait]
    impl SandboxStarter for UnreachableStarter {
        async fn start(&self, _task_id: &str) -> Result<(), SandboxError> {
            unreachable!("watching a status starts nothing")
        }
    }

    #[async_trait]
    impl SandboxStopper for UnreachableStarter {
        async fn stop(
            &self,
            _task_id: &str,
            _reason: SandboxStopReason,
        ) -> Result<(), SandboxError> {
            unreachable!("watching a status stops nothing")
        }
    }

    #[async_trait]
    impl SandboxOperator for UnreachableStarter {
        async fn restart(&self, _task_id: &str) -> Result<(), SandboxError> {
            unreachable!("watching a status restarts nothing")
        }

        async fn redeploy(&self, _task_id: &str) -> Result<(), SandboxError> {
            unreachable!("watching a status redeploys nothing")
        }

        async fn cleanup(&self, _task_id: &str) -> Result<(), SandboxError> {
            unreachable!("watching a status cleans up nothing")
        }

        /// The one method here that is reached: `detail` asks it for the image
        /// a redeploy would install. A newer tag than `MovingStatus` reports
        /// running, so the panel's "up to date" comparison has both halves.
        async fn expected_image(&self, _task_id: &str) -> Result<String, SandboxError> {
            Ok("moatless/sandbox:2".to_string())
        }
    }

    #[async_trait]
    impl ClusterDiagnostics for UnreachableStarter {
        async fn list_sandbox_diagnostics(
            &self,
        ) -> Result<Vec<moatless_ports::sandbox::SandboxDiagnosticsEntry>, SandboxError> {
            unreachable!("watching a status reads no fleet")
        }
    }

    fn viewer() -> ScopeContext {
        ScopeContext {
            user_id: "viewer".to_string(),
            is_admin: false,
        }
    }

    /// The seed is a value the client already has, so the first poll must not
    /// repeat it. Starting the change detector empty would send every idle
    /// subscriber a duplicate one interval in, and "silent unless something
    /// moved" would be false for exactly the case it is meant to cover.
    #[tokio::test(start_paused = true)]
    async fn a_status_that_has_not_moved_since_the_seed_sends_nothing() {
        let observer = Arc::new(MovingStatus::at(false));
        let (frames, mut received) = mpsc::channel(8);
        let request_id: crate::RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
        let mut streams = crate::subscription::StreamSet::new(frames);
        let watched = observer.clone();
        streams.start_produced(&request_id, None, move |sink| {
            watch_status(
                sink,
                watched_state(watched.clone()),
                viewer(),
                "task-1".to_string(),
                json!({ "sandboxStatus": "initializing" }),
            )
        });

        tokio::time::sleep(STATUS_POLL_INTERVAL * 3).await;
        assert!(
            received.try_recv().is_err(),
            "an unchanged status is not news"
        );

        observer.moved.store(true, Ordering::SeqCst);
        tokio::time::sleep(STATUS_POLL_INTERVAL * 2).await;
        let frame = received.try_recv().expect("the move is news");
        assert_eq!(frame["values"][0]["sandboxStatus"], "ready");
    }

    /// The key is present when there is an agent to report on, and *absent* —
    /// never null — when there is not. A client decodes the whole result or
    /// none of it, so a null here would cost it the lifecycle state too.
    #[tokio::test]
    async fn the_agent_is_reported_when_there_is_one_and_omitted_when_there_is_not() {
        for (agent, expected) in [
            (
                Some(moatless_ports::sandbox::AgentStatus::Running),
                Some("running"),
            ),
            (
                Some(moatless_ports::sandbox::AgentStatus::Unknown),
                Some("unknown"),
            ),
            (None, None),
        ] {
            let observer = Arc::new(match agent {
                Some(agent) => MovingStatus::reporting(agent),
                None => MovingStatus::at(true),
            });
            let payload = status(&watched_state(observer), &viewer(), "task-1")
                .await
                .expect("a readable task");

            assert_eq!(payload["sandboxStatus"], "ready");
            assert_eq!(
                payload.get("agentStatus").and_then(Value::as_str),
                expected,
                "{agent:?}"
            );
        }
    }

    /// The state this key exists for: the turn ended, so the agent is idle, and
    /// the build it registered is still running. Reading `agentStatus` alone
    /// here says "idle", which is why the commands have to travel with it.
    #[tokio::test]
    async fn a_running_command_is_reported_beside_an_idle_agent() {
        let observer = Arc::new(MovingStatus::running_commands(&["mvn verify"]));
        let payload = status(&watched_state(observer), &viewer(), "task-1")
            .await
            .expect("a readable task");

        assert_eq!(payload["agentStatus"], "idle");
        let commands = payload["commands"].as_array().expect("the commands key");
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0]["label"], "mvn verify");
        assert_eq!(commands[0]["state"], "running");
        assert_eq!(commands[0]["startedAtUnixMs"], 1_754_000_000_000_i64);
        // No elapsed field: a duration recomputed per read would make every
        // poll of `watch_status` look like a change and push forever.
        assert!(commands[0].get("elapsedSecs").is_none());
    }

    /// Absent rather than `[]`, so a client tells "this server does not report
    /// commands" from "this sandbox has none" by the capability, not by the
    /// shape of the payload.
    #[tokio::test]
    async fn no_registered_command_omits_the_key() {
        let observer = Arc::new(MovingStatus::at(true));
        let payload = status(&watched_state(observer), &viewer(), "task-1")
            .await
            .expect("a readable task");

        assert!(payload.get("commands").is_none());
    }

    /// The push exists to say when something moved. A command that starts
    /// between polls is a move, and one whose set is unchanged is not.
    #[tokio::test(start_paused = true)]
    async fn a_command_that_starts_between_polls_is_pushed() {
        let observer = Arc::new(MovingStatus::at(true));
        let state = watched_state(observer.clone());
        let seeded = status(&state, &viewer(), "task-1")
            .await
            .expect("a readable task");
        assert!(seeded.get("commands").is_none());

        let (tx, mut received) = mpsc::channel(4);
        let request_id: crate::RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
        let mut streams = crate::subscription::StreamSet::new(tx);
        streams.start_produced(&request_id, None, move |sink| {
            watch_status(sink, state, viewer(), "task-1".to_string(), seeded)
        });

        tokio::time::sleep(STATUS_POLL_INTERVAL * 2).await;
        assert!(
            received.try_recv().is_err(),
            "an unchanged status is silent"
        );

        observer.register("mvn verify");
        tokio::time::sleep(STATUS_POLL_INTERVAL * 2).await;
        let frame = received.try_recv().expect("the new command is news");
        assert_eq!(frame["values"][0]["commands"][0]["label"], "mvn verify");

        // Nothing moved since, so nothing more is sent.
        tokio::time::sleep(STATUS_POLL_INTERVAL * 2).await;
        assert!(received.try_recv().is_err(), "a steady command is silent");
    }

    /// The panel hides a section it was sent nothing for, so a runtime that
    /// reports no containers has to leave the key out rather than send an empty
    /// array. The two keys that are always there are the two the panel reads
    /// from the Task and not from the runtime.
    #[tokio::test]
    async fn a_sandbox_with_nothing_to_report_omits_every_runtime_section() {
        let payload = detail(&watched_state(Arc::new(MovingStatus::at(false))), &viewer(), "t")
            .await
            .expect("a readable task");

        assert_eq!(payload["desiredState"], "running");
        assert_eq!(payload["idleTimeoutMinutes"], 15);
        for section in ["containers", "runtimeEvents"] {
            assert!(payload.get(section).is_none(), "{section}");
        }
    }

    /// The two halves of the Image section come from two places — the running
    /// image from the status read, the expected one from the lifecycle port —
    /// so the comparison the panel gates Redeploy on is only right when both
    /// arrive. Pinned here because neither source alone can produce it.
    #[tokio::test]
    async fn a_running_sandbox_reports_both_images_and_compares_them() {
        let payload = detail(&watched_state(Arc::new(MovingStatus::at(true))), &viewer(), "t")
            .await
            .expect("a readable task");

        assert_eq!(payload["image"]["current"], "moatless/sandbox:1");
        assert_eq!(payload["image"]["latest"], "moatless/sandbox:2");
        assert_eq!(payload["image"]["upToDate"], false);
        assert_eq!(payload["containers"][0]["cpuMillicores"], 35);
        assert_eq!(payload["containers"][0]["memoryMb"], 512);
    }

    /// A sandbox that is not provisioned still has an expected image, and the
    /// panel shows it: "nothing is running, a start would give you this" is a
    /// different answer from "there is nothing to say".
    #[tokio::test]
    async fn an_unprovisioned_sandbox_still_reports_the_expected_image() {
        let payload = detail(&watched_state(Arc::new(MovingStatus::at(false))), &viewer(), "t")
            .await
            .expect("a readable task");

        assert!(payload["image"]["current"].is_null());
        assert_eq!(payload["image"]["latest"], "moatless/sandbox:2");
        assert!(payload["image"]["upToDate"].is_null());
    }
}
