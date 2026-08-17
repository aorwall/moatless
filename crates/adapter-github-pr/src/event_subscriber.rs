//! Claiming the GitHub writes an agent made through a raw shell command.
//!
//! An agent that runs `gh pr comment` or `git push` in its sandbox changes
//! GitHub without going through this Adapter, so the resulting webhook comes
//! back looking like a stranger's edit. [`shell_reconcile`](super::shell_reconcile)
//! recognises those commands in the transcript and claims them after the fact;
//! this module is the seam that gets it the transcript.
//!
//! It subscribes to `agent.message`, which is the event that carries a tool
//! result, and runs after the generic handler has persisted the message — the
//! claim is keyed on the `tool_use_id` of the row that command produced, so the
//! row has to exist first.

use std::sync::Arc;

use moatless_events::AgentSubType;
use moatless_loops::adapters::events::{TaskEventSubscriber, TaskEventSubscribers};
use moatless_loops::bindings::recently_sent::RecentlySent;
use moatless_tasks::agent_event::{AgentEvent, EventKey};
use moatless_tasks::message::dao::MessageDao;
use moatless_tasks::message::models::MessageType;
use moatless_tasks::task::dao::TaskDao;
use moatless_workspaces::repository::dao::RepositoryDao;

use super::adapter::GithubPrAdapter;

/// The ports the shell-claim reaction reads.
pub struct GithubPrEventSubscriber {
    adapter: Arc<GithubPrAdapter>,
    recently_sent: Arc<RecentlySent>,
    messages: Arc<dyn MessageDao>,
    tasks: Arc<dyn TaskDao>,
    repositories: Arc<dyn RepositoryDao>,
}

impl GithubPrEventSubscriber {
    pub fn new(
        adapter: Arc<GithubPrAdapter>,
        recently_sent: Arc<RecentlySent>,
        messages: Arc<dyn MessageDao>,
        tasks: Arc<dyn TaskDao>,
        repositories: Arc<dyn RepositoryDao>,
    ) -> Self {
        Self {
            adapter,
            recently_sent,
            messages,
            tasks,
            repositories,
        }
    }
}

#[async_trait::async_trait]
impl TaskEventSubscriber for GithubPrEventSubscriber {
    fn name(&self) -> &'static str {
        super::delivery::ADAPTER
    }

    async fn on_task_event(&self, event: &AgentEvent) {
        // Tool results arrive on the SDK's "user" role, which the conversation
        // stores as a System message. Read the role and content the same way the
        // persisting handler does, so the two agree on what a block is.
        let role = event
            .payload
            .get("message")
            .and_then(|m| m.get("role"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("assistant");
        if role != "user" {
            return;
        }
        let content = event
            .payload
            .get("message")
            .and_then(|m| m.get("content"))
            .cloned()
            .unwrap_or_else(|| serde_json::json!([]));
        let content = if let Some(text) = content.as_str() {
            serde_json::json!([{ "type": "text", "text": text }])
        } else {
            content
        };

        let targets = moatless_tasks::message::hooks::extract_shell_reconcile_targets(
            &self.messages,
            &event.task_id,
            MessageType::System,
            &content,
        )
        .await;

        for target in targets {
            super::shell_reconcile::reconcile_shell_command(
                Some(&self.adapter),
                &self.recently_sent,
                self.messages.as_ref(),
                self.tasks.as_ref(),
                self.repositories.as_ref(),
                &event.task_id,
                &target.tool_use_id,
                &target.cmd,
                &target.result_text,
            )
            .await;
        }
    }
}

/// Subscribe the GitHub PR shell claim to the events it needs.
pub fn subscribe(
    subscribers: &mut TaskEventSubscribers,
    subscriber: Arc<GithubPrEventSubscriber>,
) {
    subscribers.subscribe([EventKey::Agent(AgentSubType::Message)], subscriber);
}
