//! Subscribing to a Sandbox's status, which answers from the last observation
//! rather than waiting for a poll.

use super::fixtures::*;
use super::*;

/// An observer that reports one fixed status, for the subscribe test below.
struct FixedStatus(SandboxStatus);

#[async_trait]
impl SandboxIntentReader for FixedStatus {
    async fn desired_state(&self, _task_id: &str) -> Result<SandboxDesiredState, SandboxError> {
        Ok(SandboxDesiredState::Running)
    }
}

#[async_trait]
impl SandboxObserver for FixedStatus {
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
            sandbox_status: self.0,
            agent_status: None,
            sandbox_error: None,
            commands: Vec::new(),
        })
    }

    async fn detail(
        &self,
        _task_id: &str,
        _task_status: TaskStatus,
        _desired_state: SandboxDesiredState,
    ) -> Result<moatless_ports::sandbox::SandboxDetail, SandboxError> {
        unreachable!("the lifecycle read reads no detail")
    }
}

fn state_with_sandbox_status(status: SandboxStatus) -> UiRpcState {
    let mut state = state();
    let mut tasks = MockT3TaskDao::new();
    tasks.expect_find_by_id().returning(|_, _| {
        Ok(Some(TaskView {
            status: TaskStatus::Open,
            ttl_minutes: 15,
        }))
    });
    state.sandbox.tasks = Arc::new(tasks);
    state.sandbox.observer = Arc::new(FixedStatus(status));
    state.sandbox.intent = Arc::new(FixedStatus(status));
    state
}

/// The subscription seeds before its first poll interval elapses; without it,
/// the indicator renders nothing until the first poll — the stale-status bug
/// this push channel exists to end.
#[tokio::test]
async fn subscribing_to_a_sandbox_status_answers_before_the_first_poll() {
    let frame = request(
        "1",
        "sandbox.subscribeStatus",
        json!({ "threadId": "task-1" }),
    );
    let mut session = session();
    let output = session
        .handle_json(
            &frame,
            &state_with_sandbox_status(SandboxStatus::Ready),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["_tag"], "Chunk");
    assert_eq!(output.frames[0]["values"][0]["sandboxStatus"], "ready");
}

/// A subscription is per-thread, so the thread is not optional. Left to the
/// poller this would be a stream that authorizes nothing and never emits.
#[tokio::test]
async fn subscribing_to_a_sandbox_status_without_a_thread_is_refused() {
    let frame = request("1", "sandbox.subscribeStatus", json!({}));
    let mut session = session();
    let output = session.handle_json(&frame, &state(), &viewer()).await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
    assert!(
        output.frames[0]["exit"]["cause"][0]["defect"]
            .as_str()
            .unwrap_or_default()
            .contains("threadId"),
        "the refusal should name what is missing, got {output:?}"
    );
}

/// `sandbox.start` shares its missing-`threadId` prologue with every other
/// thread-addressed method through `Request::require_thread_id`; this pins
/// the exact defect text so that sharing cannot blur which method refused.
#[tokio::test]
async fn starting_a_sandbox_without_a_thread_is_refused_by_name() {
    let frame = request("1", "sandbox.start", json!({}));
    let mut session = session();
    let output = session.handle_json(&frame, &state(), &viewer()).await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
    assert_eq!(
        output.frames[0]["exit"]["cause"][0]["defect"],
        "sandbox.start requires a threadId"
    );
}
