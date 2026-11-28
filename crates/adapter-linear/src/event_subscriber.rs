//! Mirroring a task's own event stream into its Linear agent session.
//!
//! Linear's UI shows a session as a live feed, so a bound Task's progress —
//! assistant output, tool runs, sandbox status, turn end — is pushed here as it
//! happens.
//!
//! [`ActiveLinearTurns`], owned here as the only reader, gates emission (see
//! its own doc); every emission is a detached, best-effort `tokio::spawn`.

use std::sync::Arc;

use moatless_adapter_model::apps::AdapterAppReader;
use moatless_adapter_model::connections::ConnectionLookup;
use moatless_core::config::Config;
use moatless_loops::adapters::events::{TaskEventSubscriber, TaskEventSubscribers};
use moatless_loops::bindings::operations::Bindings;
use moatless_ports::identity::AppCredentials;
use moatless_ports::task::{AssistantReply, TaskReader, TranscriptAppend};
use moatless_tasks::agent_event::{AgentEvent, EventKey};
use moatless_types::agent::AgentSubType;
use moatless_types::message::MessageSubType;
use moatless_types::sandbox::SandboxStatus;

use super::active_turns::ActiveLinearTurns;
use super::activity::{
    ActivityKind, MIN_THOUGHT_CHARS, MessagePost, message_posts, notify_linear_action,
    notify_linear_message_posts, notify_linear_status,
};

/// The ports the Linear event reaction reads.
pub struct LinearEventSubscriber {
    app_tokens: Arc<dyn AppCredentials>,
    task_bindings: Arc<Bindings>,
    adapter_connections: Arc<dyn ConnectionLookup>,
    config: Arc<Config>,
    apps: Arc<dyn AdapterAppReader>,
    messages: Arc<dyn TranscriptAppend>,
    tasks: Arc<dyn TaskReader>,
    active_turns: Arc<ActiveLinearTurns>,
}

impl LinearEventSubscriber {
    #[expect(
        clippy::too_many_arguments,
        reason = "the ports this subscriber's reaction reads, inverted out of the AppState \
                  it used to hold; grouping them back would reintroduce that bag"
    )]
    pub fn new(
        app_tokens: Arc<dyn AppCredentials>,
        task_bindings: Arc<Bindings>,
        adapter_connections: Arc<dyn ConnectionLookup>,
        config: Arc<Config>,
        apps: Arc<dyn AdapterAppReader>,
        messages: Arc<dyn TranscriptAppend>,
        tasks: Arc<dyn TaskReader>,
        active_turns: Arc<ActiveLinearTurns>,
    ) -> Self {
        Self {
            app_tokens,
            task_bindings,
            adapter_connections,
            config,
            apps,
            messages,
            tasks,
            active_turns,
        }
    }

    /// The events this Adapter reacts to.
    fn keys() -> [EventKey; 9] {
        [
            EventKey::Agent(AgentSubType::Started),
            EventKey::Message(MessageSubType::Upsert),
            EventKey::Agent(AgentSubType::Completed),
            EventKey::Agent(AgentSubType::Stopped),
            EventKey::Agent(AgentSubType::Error),
            EventKey::Sandbox(SandboxStatus::Error),
            EventKey::Sandbox(SandboxStatus::Initializing),
            EventKey::Sandbox(SandboxStatus::Ready),
            EventKey::Sandbox(SandboxStatus::Stopped),
        ]
    }

    /// Assistant text and tool calls as the Projector writes them; tool
    /// results resolved back to the tool that produced them.
    fn on_message_upsert(&self, event: &AgentEvent) {
        let posts = fresh_posts(&self.active_turns, &event.task_id, &event.payload);
        if posts.is_empty() {
            return;
        }
        tokio::spawn(notify_linear_message_posts(
            self.config.http.linear,
            self.app_tokens.clone(),
            self.task_bindings.clone(),
            self.adapter_connections.clone(),
            self.apps.clone(),
            self.messages.clone(),
            event.task_id.clone(),
            posts,
        ));
    }

    /// A terminal `response` activity carrying the last assistant message.
    ///
    /// Awaits the message read rather than spawning it, since the body must
    /// resolve before the gate closes; only the Linear call is detached.
    async fn on_agent_completed(&self, event: &AgentEvent) {
        // An `is_error` completion is routed through the error machinery by the
        // generic handler and posts no response activity — only the gate closes.
        let is_error = event
            .payload
            .get("isError")
            .or_else(|| event.payload.get("is_error"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if !is_error {
            self.emit_completion_response(&event.task_id, event.turn_number)
                .await;
        }
        self.active_turns.close(&event.task_id);
    }

    async fn emit_completion_response(&self, task_id: &str, turn: Option<i32>) {
        let msg = match self.messages.last_assistant_reply(task_id).await {
            Ok(Some(m)) => m,
            Ok(None) => {
                tracing::warn!(
                    task_id,
                    "agent.completed: no assistant message found; skipping Linear response activity"
                );
                return;
            }
            Err(e) => {
                tracing::warn!(
                    task_id,
                    error = %e,
                    "agent.completed: failed to fetch last assistant message; skipping Linear response activity"
                );
                return;
            }
        };

        let Some(mut body) = completion_text(&self.active_turns, task_id, turn, &msg) else {
            tracing::info!(
                task_id,
                turn,
                message_uuid = %msg.uuid,
                "agent.completed: the turn has no unposted final text; skipping Linear response activity"
            );
            return;
        };

        // Append the `Moatless task:` footer so `handle_comment_created`'s
        // echo filter can recover the authoring task id and drop this
        // comment when Linear webhooks it back. Without it, the response
        // re-enters as a fresh inbound comment, the agent replies, and the
        // task loops forever.
        if let Some(footer) = moatless_loops::bindings::footer::task_comment_footer(
            self.tasks.as_ref(),
            &self.config.auth.frontend_url,
            task_id,
        )
        .await
        {
            body.push_str(&footer);
        }

        self.spawn_status(task_id, ActivityKind::Response, body);
    }

    /// A persistent `error` activity, so a mid-flight failure is visible in the
    /// session rather than only in the task.
    fn on_failure(&self, event: &AgentEvent, fallback: &str) {
        if self.active_turns.is_open(&event.task_id) {
            let body = event
                .payload
                .get("error")
                .and_then(serde_json::Value::as_str)
                .or_else(|| {
                    event
                        .payload
                        .get("message")
                        .and_then(serde_json::Value::as_str)
                })
                .unwrap_or(fallback)
                .to_string();
            self.spawn_status(&event.task_id, ActivityKind::Error, body);
        }
        self.active_turns.close(&event.task_id);
    }

    /// A sandbox or agent status marker, as an ephemeral `action` so each new
    /// status replaces the previous one in Linear's UI instead of piling up.
    fn spawn_action(&self, task_id: &str, action: &str, parameter: &str) {
        tokio::spawn(notify_linear_action(
            self.config.http.linear,
            self.app_tokens.clone(),
            self.task_bindings.clone(),
            self.adapter_connections.clone(),
            self.apps.clone(),
            task_id.to_string(),
            action.to_string(),
            parameter.to_string(),
            None,
            true,
        ));
    }

    fn spawn_status(&self, task_id: &str, kind: ActivityKind, body: String) {
        tokio::spawn(notify_linear_status(
            self.config.http.linear,
            self.app_tokens.clone(),
            self.task_bindings.clone(),
            self.adapter_connections.clone(),
            self.apps.clone(),
            task_id.to_string(),
            kind,
            body,
            false,
        ));
    }
}

#[async_trait::async_trait]
impl TaskEventSubscriber for LinearEventSubscriber {
    fn name(&self) -> &'static str {
        super::delivery::ADAPTER
    }

    async fn on_task_event(&self, event: &AgentEvent) {
        match event.event_key() {
            // Re-opens the gate, so a manual `moat send` against a Linear-bound
            // task re-surfaces events in Linear on the second and later turns.
            EventKey::Agent(AgentSubType::Started) => {
                self.active_turns.open(&event.task_id);
                self.spawn_action(&event.task_id, "Agent", AgentSubType::Started.as_str());
            }
            EventKey::Message(MessageSubType::Upsert) => self.on_message_upsert(event),
            EventKey::Agent(AgentSubType::Completed) => self.on_agent_completed(event).await,
            // A user-initiated stop emits no final activity; the gate close just
            // keeps post-stop stragglers out.
            EventKey::Agent(AgentSubType::Stopped) => self.active_turns.close(&event.task_id),
            EventKey::Agent(AgentSubType::Error) => self.on_failure(event, "Agent error"),
            EventKey::Sandbox(SandboxStatus::Error) => self.on_failure(event, "Sandbox error"),
            EventKey::Sandbox(status) if self.active_turns.is_open(&event.task_id) => {
                self.spawn_action(&event.task_id, "Sandbox", status.as_str());
            }
            _ => {}
        }
    }
}

/// The posts an upserted Message owes that its open turn has not posted.
fn fresh_posts(
    turns: &ActiveLinearTurns,
    task_id: &str,
    message: &serde_json::Value,
) -> Vec<MessagePost> {
    message_posts(message)
        .into_iter()
        .filter_map(|post| match post {
            MessagePost::Thought { key, text } => turns
                .claim_text(task_id, &key, &text, MIN_THOUGHT_CHARS)
                .map(|text| MessagePost::Thought { key, text }),
            post => turns.claim(task_id, post.key()).then_some(post),
        })
        .collect()
}

/// The text `reply` posts as the `response` ending `turn`, or `None` when the
/// turn wrote no unposted final text. `reply` is the Task's latest, so without
/// this a turn ending empty reposts an earlier turn's reply.
fn completion_text(
    turns: &ActiveLinearTurns,
    task_id: &str,
    turn: Option<i32>,
    reply: &AssistantReply,
) -> Option<String> {
    // Lower only: a batched turn's reply is written to the latest sibling turn.
    let earlier_turn = matches!((reply.turn_number, turn), (Some(r), Some(t)) if r < t);
    let text = reply.text.as_deref().filter(|t| !t.trim().is_empty())?;
    (!earlier_turn && turns.claim_response(task_id, &reply.uuid)).then(|| text.to_string())
}

/// Subscribe the Linear reaction to the events it needs.
///
/// One call from the composition root; a build without it leaves Linear
/// sessions without live progress and changes nothing else.
pub fn subscribe(subscribers: &mut TaskEventSubscribers, subscriber: Arc<LinearEventSubscriber>) {
    subscribers.subscribe(LinearEventSubscriber::keys(), subscriber);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upsert() -> serde_json::Value {
        serde_json::json!({
            "uuid": "msg_1",
            "messageType": "assistant",
            "content": [
                { "type": "text", "text": "Reading the config." },
                { "type": "tool_use", "toolUseId": "toolu_1", "toolName": "Read",
                  "toolInput": { "file_path": "/repo/a.rs" } },
            ],
            "toolResults": [{ "toolUseId": "toolu_1", "content": "fn a() {}" }],
        })
    }

    #[test]
    fn a_replayed_upsert_posts_nothing() {
        let turns = ActiveLinearTurns::new();
        turns.open("task-1");
        let keys: Vec<String> = fresh_posts(&turns, "task-1", &upsert())
            .iter()
            .map(|p| p.key().to_string())
            .collect();
        assert_eq!(keys, ["text:msg_1", "call:toolu_1", "result:toolu_1"]);
        assert!(fresh_posts(&turns, "task-1", &upsert()).is_empty());
    }

    /// Claude's later text blocks fold into the same Message, so its text
    /// grows across upserts; each addition posts once.
    #[test]
    fn a_growing_thought_posts_each_addition_once() {
        let turns = ActiveLinearTurns::new();
        turns.open("task-1");
        let thoughts = |message: &serde_json::Value| -> Vec<String> {
            fresh_posts(&turns, "task-1", message)
                .into_iter()
                .filter_map(|post| match post {
                    MessagePost::Thought { text, .. } => Some(text),
                    _ => None,
                })
                .collect()
        };
        let mut message = upsert();
        assert_eq!(thoughts(&message), ["Reading the config."]);
        message["content"][0]["text"] = "Reading the config.\n\nIt sets the port.".into();
        assert_eq!(thoughts(&message), ["It sets the port."]);
        assert!(thoughts(&message).is_empty());
    }

    fn reply(uuid: &str, turn: i32, text: &str) -> AssistantReply {
        AssistantReply {
            uuid: uuid.to_string(),
            text: Some(text.to_string()),
            turn_number: Some(turn),
        }
    }

    /// Turn 2 ends empty, so the latest reply is still turn 1's.
    #[test]
    fn a_turn_ending_empty_does_not_repost_the_previous_reply() {
        let turns = ActiveLinearTurns::new();
        let fix_ready = reply("msg_1", 1, "Fix is ready.");
        let responses: Vec<String> = [(1, &fix_ready), (2, &fix_ready), (3, &fix_ready)]
            .into_iter()
            .filter_map(|(turn, r)| completion_text(&turns, "task-1", Some(turn), r))
            .collect();
        assert_eq!(responses, ["Fix is ready."]);
    }

    #[test]
    fn a_replayed_completion_or_an_unnumbered_one_posts_a_reply_once() {
        let turns = ActiveLinearTurns::new();
        let r = reply("msg_1", 1, "Done.");
        assert!(completion_text(&turns, "task-1", Some(1), &r).is_some());
        assert!(completion_text(&turns, "task-1", Some(1), &r).is_none());
        assert!(completion_text(&turns, "task-1", None, &r).is_none());
        let next = reply("msg_2", 2, "Done again.");
        assert!(completion_text(&turns, "task-1", None, &next).is_some());
    }

    #[test]
    fn a_reply_written_to_a_later_batched_turn_still_posts() {
        let turns = ActiveLinearTurns::new();
        let r = reply("msg_1", 3, "Handled both.");
        assert!(completion_text(&turns, "task-1", Some(2), &r).is_some());
    }

    #[test]
    fn an_empty_reply_posts_nothing() {
        let turns = ActiveLinearTurns::new();
        assert!(completion_text(&turns, "task-1", Some(1), &reply("msg_1", 1, " \n")).is_none());
    }

    #[test]
    fn a_closed_turn_posts_nothing() {
        assert!(fresh_posts(&ActiveLinearTurns::new(), "task-1", &upsert()).is_empty());
    }

    #[test]
    fn a_subagent_or_user_message_posts_nothing() {
        let mut subagent = upsert();
        subagent["parentUuid"] = "toolu_0".into();
        let mut user = upsert();
        user["messageType"] = "user".into();
        assert!(message_posts(&subagent).is_empty());
        assert!(message_posts(&user).is_empty());
    }
}
