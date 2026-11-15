//! A person's Stop and Force stop of an agent, against scripted stores.

use std::sync::{Arc, Mutex};

use moatless_core::events::test_support::{RecordedEvent, RecordingPublisher};
use moatless_ports::sandbox::{AgentStopOutcome, SandboxLifecycle, SandboxOpError};
use moatless_types::agent::AgentSubType;
use moatless_types::sandbox::{ArchiveRetention, SandboxDesiredState, SandboxStopReason};

use super::*;
use crate::task::dao::MockTaskDao;
use crate::task::test_support::make_task_row;
use crate::task_turn::dao::MockTaskTurnDao;

const OWNER: &str = "11111111-1111-1111-1111-111111111111";

/// Answers `stop_agent` with `outcome` and records which calls reached it.
struct FakeSandbox {
    outcome: AgentStopOutcome,
    calls: Mutex<Vec<&'static str>>,
}

impl FakeSandbox {
    fn answering(outcome: AgentStopOutcome) -> Self {
        Self {
            outcome,
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<&'static str> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl SandboxLifecycle for FakeSandbox {
    async fn start(&self, _task_id: &str) -> Result<(), SandboxOpError> {
        unreachable!("no stop starts a sandbox")
    }
    async fn stop(&self, _task_id: &str, _reason: SandboxStopReason) -> Result<(), SandboxOpError> {
        self.calls.lock().unwrap().push("stop");
        Ok(())
    }
    async fn stop_agent(&self, _task_id: &str, _reason: SandboxStopReason) -> AgentStopOutcome {
        self.calls.lock().unwrap().push("stop_agent");
        self.outcome
    }
    async fn remove(
        &self,
        _task_id: &str,
        _retention: ArchiveRetention,
    ) -> Result<(), SandboxOpError> {
        unreachable!("no stop removes a sandbox")
    }
    async fn archive_storage(&self, _task_id: &str) -> Result<(), SandboxOpError> {
        unreachable!("no stop archives storage")
    }
    async fn stopped_with_storage(
        &self,
        _task_ids: &[String],
    ) -> Result<std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>, SandboxOpError>
    {
        unreachable!("no stop lists stopped sandboxes")
    }
    async fn desired_state(&self, _task_id: &str) -> Result<SandboxDesiredState, SandboxOpError> {
        unreachable!("no stop reads desired state")
    }
}

/// A store holding one writable `task-1`, whose sandbox also holds `members`.
fn tasks(members: &'static [&'static str]) -> TaskCommands {
    let mut store = MockTaskDao::new();
    store
        .expect_find_by_id()
        .returning(|task_id, _| Ok(Some(make_task_row(task_id, OWNER))));
    store.expect_list_sandbox_members().returning(|sandboxes| {
        assert_eq!(sandboxes, ["task-1".to_string()]);
        Ok(members
            .iter()
            .map(|member| (member.to_string(), "task-1".to_string()))
            .collect())
    });
    TaskCommands::new(Arc::new(store))
}

fn owner() -> ScopeContext {
    ScopeContext::new(OWNER.to_string(), false)
}

fn stops(events: &RecordingPublisher) -> usize {
    events
        .events()
        .iter()
        .filter(|event| **event == RecordedEvent::Agent(AgentSubType::Stopped))
        .count()
}

#[tokio::test]
async fn a_confirmed_stop_closes_the_turn_and_stamps_nothing() {
    let mut turns = MockTaskTurnDao::new();
    turns.expect_mark_stop_unresponsive().never();
    let turns = TaskTurns::new(Arc::new(turns));
    let sandbox = FakeSandbox::answering(AgentStopOutcome::Stopped);
    let events = RecordingPublisher::new();

    let outcome = stop_agent(
        &tasks(&[]),
        &turns,
        &sandbox,
        Some(&events),
        &owner(),
        "task-1",
    )
    .await
    .unwrap();

    assert_eq!(outcome, AgentStopOutcome::Stopped);
    assert_eq!(stops(&events), 1);
}

/// The agent is still working, so its turn stays open: closing it would let a
/// send start a second agent beside the one that ignored the Stop.
#[tokio::test]
async fn an_unconfirmed_stop_stamps_the_turn_and_leaves_it_open() {
    let mut turns = MockTaskTurnDao::new();
    turns
        .expect_mark_stop_unresponsive()
        .withf(|task_id, _| task_id == "task-1")
        .times(1)
        .returning(|_, _| Ok(true));
    let turns = TaskTurns::new(Arc::new(turns));
    let sandbox = FakeSandbox::answering(AgentStopOutcome::Unresponsive);
    let events = RecordingPublisher::new();

    let outcome = stop_agent(
        &tasks(&[]),
        &turns,
        &sandbox,
        Some(&events),
        &owner(),
        "task-1",
    )
    .await
    .unwrap();

    assert_eq!(outcome, AgentStopOutcome::Unresponsive);
    assert_eq!(stops(&events), 0);
}

#[tokio::test]
async fn a_force_stop_stops_the_sandbox_then_closes_the_turn() {
    let mut turns = MockTaskTurnDao::new();
    turns.expect_latest_turn_number().returning(|_| Ok(Some(3)));
    let turns = TaskTurns::new(Arc::new(turns));
    let sandbox = FakeSandbox::answering(AgentStopOutcome::Unresponsive);
    let events = RecordingPublisher::new();

    let outcome = force_stop_agent(
        &tasks(&["task-1"]),
        &turns,
        &sandbox,
        Some(&events),
        &owner(),
        "task-1",
        Some(3),
    )
    .await
    .unwrap();

    assert_eq!(outcome, ForceStop::Released);
    assert_eq!(sandbox.calls(), ["stop"]);
    assert_eq!(stops(&events), 1);
}

#[tokio::test]
async fn a_force_stop_of_a_shared_sandbox_closes_only_this_turn() {
    let turns = TaskTurns::new(Arc::new(MockTaskTurnDao::new()));
    let sandbox = FakeSandbox::answering(AgentStopOutcome::Unresponsive);
    let events = RecordingPublisher::new();

    let outcome = force_stop_agent(
        &tasks(&["task-1", "fork-1"]),
        &turns,
        &sandbox,
        Some(&events),
        &owner(),
        "task-1",
        None,
    )
    .await
    .unwrap();

    assert_eq!(outcome, ForceStop::Shared(vec!["fork-1".to_string()]));
    assert!(
        sandbox.calls().is_empty(),
        "the other member's sandbox stays up"
    );
    assert_eq!(stops(&events), 1);
}

#[tokio::test]
async fn a_force_stop_after_a_newer_turn_started_stops_nothing() {
    let mut turns = MockTaskTurnDao::new();
    turns.expect_latest_turn_number().returning(|_| Ok(Some(4)));
    let turns = TaskTurns::new(Arc::new(turns));
    let sandbox = FakeSandbox::answering(AgentStopOutcome::Stopped);
    let events = RecordingPublisher::new();

    let outcome = force_stop_agent(
        &tasks(&["task-1"]),
        &turns,
        &sandbox,
        Some(&events),
        &owner(),
        "task-1",
        Some(3),
    )
    .await
    .unwrap();

    assert_eq!(outcome, ForceStop::Superseded);
    assert!(sandbox.calls().is_empty());
    assert_eq!(stops(&events), 0);
}
