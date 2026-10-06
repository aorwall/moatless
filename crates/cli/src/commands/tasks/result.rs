//! `moat tasks status --result`: where a task's work stands, and what it last
//! said.

use serde::Serialize;

use crate::client::{ClientError, MoatClient};
use crate::output;
use moatless_api_schemas::enums::{MessageRole, TaskStatus};
use moatless_api_schemas::message::UiMessage;
use moatless_api_schemas::sandbox::{AgentStatus, LiveStatus};
use moatless_api_schemas::task::ListTasksQuery;

/// How many of the newest Messages are searched for the latest result.
const RESULT_SEARCH_WINDOW: i32 = 50;

/// Where a task's work stands, most pressing first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum WorkState {
    Closed,
    Errored,
    /// The agent is mid-turn.
    Working,
    /// The agent is parked on a person's answer.
    WaitingForInput,
    /// The turn ended, and a `moat cmd` command it started still runs.
    RunningCommands,
    /// The turn ended, and a created-by child is mid-turn or parked.
    WaitingForChildren,
    /// Nothing runs; the latest result is final until the next message.
    Done,
}

impl WorkState {
    fn label(self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Errored => "errored",
            Self::Working => "working",
            Self::WaitingForInput => "waiting for input",
            Self::RunningCommands => "running commands",
            Self::WaitingForChildren => "waiting for children",
            Self::Done => "done",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskResult {
    task_id: String,
    work_state: WorkState,
    agent_status: Option<AgentStatus>,
    /// Children whose agent is mid-turn or parked.
    active_child_ids: Vec<String>,
    latest_result: Option<LatestResult>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LatestResult {
    turn_number: Option<i32>,
    timestamp: String,
    text: String,
}

fn agent_in_turn(live: &LiveStatus) -> bool {
    matches!(
        live.agent_status,
        Some(AgentStatus::Running | AgentStatus::Waiting)
    )
}

fn work_state(task_status: TaskStatus, live: &LiveStatus, active_children: bool) -> WorkState {
    match (task_status, live.agent_status) {
        (TaskStatus::Closed, _) => WorkState::Closed,
        (_, Some(AgentStatus::Running)) => WorkState::Working,
        (_, Some(AgentStatus::Waiting)) => WorkState::WaitingForInput,
        (TaskStatus::Error, _) => WorkState::Errored,
        _ if live.commands.iter().any(|c| c.state.is_running()) => WorkState::RunningCommands,
        _ if active_children => WorkState::WaitingForChildren,
        _ => WorkState::Done,
    }
}

/// The newest assistant Message carrying text, its text blocks joined.
fn latest_result(messages: &[UiMessage]) -> Option<LatestResult> {
    messages
        .iter()
        .filter(|m| m.role == MessageRole::Assistant)
        .filter_map(|m| {
            let text = m
                .content
                .iter()
                .filter(|block| block.block_type == "text")
                .filter_map(|block| block.text.as_deref())
                .filter(|text| !text.trim().is_empty())
                .collect::<Vec<_>>()
                .join("\n\n");
            (!text.is_empty()).then(|| LatestResult {
                turn_number: m.turn_number,
                timestamp: m.timestamp.clone(),
                text,
            })
        })
        .max_by(|a, b| a.timestamp.cmp(&b.timestamp))
}

pub(super) async fn show_result(
    client: &MoatClient,
    task_id: &str,
    json: bool,
) -> Result<(), ClientError> {
    let api = client.api();
    let task = api.get_task(task_id).await?;
    let live = api.live_status(task_id).await?;

    let children = api
        .list_tasks(&ListTasksQuery {
            // The server's page ceiling; a parent past it under-reports its
            // active children.
            limit: Some(200),
            offset: None,
            status: Some(TaskStatus::Open),
            repository_id: None,
            created_by: None,
            tag: None,
            parent_task_id: Some(task_id.to_string()),
            followed_by: None,
        })
        .await?;
    let mut active_child_ids = Vec::new();
    for child in children.items {
        // A child whose status cannot be read counts as idle rather than
        // failing the whole read.
        if let Ok(child_live) = api.live_status(&child.task_id).await {
            if agent_in_turn(&child_live) {
                active_child_ids.push(child.task_id);
            }
        }
    }

    let messages = api
        .get_messages_page(task_id, true, Some(RESULT_SEARCH_WINDOW), None)
        .await?;
    let result = TaskResult {
        task_id: task_id.to_string(),
        work_state: work_state(task.status, &live, !active_child_ids.is_empty()),
        agent_status: live.agent_status,
        latest_result: latest_result(&messages.messages),
        active_child_ids,
    };

    if json {
        output::print_json_typed(&result);
        return Ok(());
    }
    println!("Work:        {}", result.work_state.label());
    let agent = live
        .agent_status
        .map_or_else(|| "n/a".to_string(), |status| status.to_string());
    println!("Agent:       {agent}");
    if !result.active_child_ids.is_empty() {
        println!("Children:    {}", result.active_child_ids.join(", "));
    }
    match result.latest_result {
        Some(latest) => {
            let turn = latest
                .turn_number
                .map_or_else(String::new, |t| format!(" (turn {t})"));
            println!(
                "\nLatest result{turn}, {}:\n{}",
                latest.timestamp, latest.text
            );
        }
        None => println!("\nNo result yet."),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use moatless_api_schemas::sandbox::SandboxStatus;

    fn live(agent_status: Option<AgentStatus>) -> LiveStatus {
        LiveStatus {
            sandbox_status: SandboxStatus::Ready,
            agent_status,
            sandbox_error: None,
            commands: Vec::new(),
        }
    }

    #[test]
    fn a_running_agent_outranks_waiting_children() {
        let state = work_state(TaskStatus::Open, &live(Some(AgentStatus::Running)), true);
        assert_eq!(state, WorkState::Working);
    }

    #[test]
    fn an_idle_agent_with_a_busy_child_waits_for_children() {
        let state = work_state(TaskStatus::Open, &live(Some(AgentStatus::Idle)), true);
        assert_eq!(state, WorkState::WaitingForChildren);
    }

    #[test]
    fn a_closed_task_is_closed_whatever_its_agent_reports() {
        let state = work_state(TaskStatus::Closed, &live(Some(AgentStatus::Running)), false);
        assert_eq!(state, WorkState::Closed);
    }

    #[test]
    fn an_idle_agent_alone_is_done() {
        let state = work_state(TaskStatus::Open, &live(None), false);
        assert_eq!(state, WorkState::Done);
    }
}
