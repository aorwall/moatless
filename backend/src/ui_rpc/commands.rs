//! `orchestration.dispatchCommand` — the one write the T3 client makes.
//!
//! T3's command union is much wider than Moatless. This module is the table
//! that says, for each command, whether it is carried out, discarded, or
//! refused, and it is deliberately explicit about all three: a command that is
//! silently discarded and a command that is refused look identical from here and
//! completely different to the person holding the mouse.
//!
//! Three commands are **discarded on purpose**:
//!
//! - `thread.runtime-mode.set` — T3's access mode has no Moatless meaning.
//!   Refusing would put an error behind a dropdown that, from the person's side,
//!   simply does nothing.
//! - `thread.interaction-mode.set` — the Build/Plan choice is carried on the
//!   turn itself and applied there, so the separate command is redundant rather
//!   than meaningful.
//! - `bootstrap.prepareWorktree` / `runSetupScript` on a turn — worktrees are
//!   the Sandbox's business, not the client's.
//!
//! Everything refused is refused because it cannot be honoured, not because it
//! has not been written yet: closing a Task from T3 is its own change,
//! Workspaces are not authored from here, and approvals and checkpoints are
//! never projected in the first place, so no control raises them.

use serde_json::{json, Value};

use moatless_backend_core::scope::ScopeContext;

use super::dao::{Conversation, ConversationDao};
use super::ports::{
    AgentCatalogSource, CommandRefusal, NewThread, QuestionAnswer, ThreadCommandSink, ThreadMeta,
    ThreadTurn,
};
use super::providers;
use crate::agent::catalog;
use crate::shared_models::AgentMode;

/// What the surface needs in order to answer a command.
pub struct CommandContext<'a> {
    pub conversations: &'a dyn ConversationDao,
    pub agents: &'a dyn AgentCatalogSource,
    pub sink: &'a dyn ThreadCommandSink,
}

/// Answer one dispatched command.
///
/// The returned value is the contract's `DispatchResult`. `sequence` is the
/// listing position the deployment has reached, so a client can tell whether the
/// change it just caused is already reflected in what it holds.
pub async fn dispatch(
    ctx: &CommandContext<'_>,
    viewer: &ScopeContext,
    command: &Value,
) -> Result<Value, CommandRefusal> {
    let tag = command
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();

    match tag.as_str() {
        "thread.turn.start" => turn_start(ctx, viewer, command).await?,
        "thread.create" => create_thread(ctx, viewer, command).await?,
        "thread.meta.update" => meta_update(ctx, viewer, command).await?,
        "thread.turn.interrupt" | "thread.session.stop" => {
            ctx.sink.stop(viewer, thread_id(command)?).await?;
        }
        "thread.user-input.respond" => respond(ctx, viewer, command).await?,

        // Accepted and discarded — see the module comment.
        "thread.runtime-mode.set" | "thread.interaction-mode.set" => {}

        "thread.delete" | "thread.archive" | "thread.unarchive" => {
            return Err(CommandRefusal::Unsupported {
                command: tag,
                reason: "Close a task from the Moatless UI instead.".to_string(),
            })
        }
        "project.create" | "project.meta.update" | "project.delete" => {
            return Err(CommandRefusal::Unsupported {
                command: tag,
                reason: "Projects are Moatless workspaces and are managed there.".to_string(),
            })
        }
        "thread.approval.respond" | "thread.checkpoint.revert" => {
            return Err(CommandRefusal::Unsupported {
                command: tag,
                reason: "Approvals and checkpoints are not part of this server.".to_string(),
            })
        }
        "thread.settle" | "thread.unsettle" | "thread.snooze" | "thread.unsnooze" => {
            return Err(CommandRefusal::Unsupported {
                command: tag,
                reason: "Settling and snoozing are not part of this server.".to_string(),
            })
        }
        other => {
            return Err(CommandRefusal::Unsupported {
                command: other.to_string(),
                reason: "This server does not implement that action.".to_string(),
            })
        }
    }

    let sequence = ctx
        .conversations
        .listing_head()
        .await
        .map(|head| head.position)
        .unwrap_or_default();
    Ok(json!({ "sequence": sequence.max(0) }))
}

/// A turn, with or without the create that T3 folds into the same frame.
async fn turn_start(
    ctx: &CommandContext<'_>,
    viewer: &ScopeContext,
    command: &Value,
) -> Result<(), CommandRefusal> {
    let task_id = thread_id(command)?.to_string();
    let message = command.get("message").ok_or_else(|| missing("message"))?;
    let text = message
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let message_uuid = message
        .get("messageId")
        .and_then(Value::as_str)
        .ok_or_else(|| missing("message.messageId"))?
        .to_string();
    let agent_mode = interaction_mode(command);

    let bootstrap = command
        .get("bootstrap")
        .and_then(|b| b.get("createThread"));

    if let Some(create) = bootstrap {
        // A brand-new thread: the instance the person picked decides which
        // agent runs it, because there is no task yet to disagree with.
        let agents = ctx.agents.list(viewer).await;
        let instance = selected_instance(command)
            .or_else(|| selected_instance(create))
            .unwrap_or(providers::CLAUDE_INSTANCE.to_string());
        let model = translate_model(&agents, &instance, command)?
            .or(translate_model(&agents, &instance, create)?);

        return ctx
            .sink
            .create_thread(
                viewer,
                NewThread {
                    task_id: task_id.clone(),
                    workspace_id: create
                        .get("projectId")
                        .and_then(Value::as_str)
                        .ok_or_else(|| missing("bootstrap.createThread.projectId"))?
                        .to_string(),
                    instance,
                    title: create
                        .get("title")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    model: model.clone(),
                    first_message: Some(ThreadTurn {
                        task_id,
                        text,
                        message_uuid,
                        model,
                        agent_mode,
                    }),
                },
            )
            .await;
    }

    let conversation = require_writable(ctx, viewer, &task_id).await?;
    let agents = ctx.agents.list(viewer).await;
    let model = model_for_existing(&agents, &conversation, command)?;
    ctx.sink
        .send_turn(
            viewer,
            ThreadTurn {
                task_id,
                text,
                message_uuid,
                model,
                agent_mode,
            },
        )
        .await
}

/// A thread created with nothing in it. Reachable only from a proposed plan,
/// which this surface never projects — implemented so the client's own "new
/// chat" path is not the one thing that fails.
async fn create_thread(
    ctx: &CommandContext<'_>,
    viewer: &ScopeContext,
    command: &Value,
) -> Result<(), CommandRefusal> {
    let agents = ctx.agents.list(viewer).await;
    let instance = selected_instance(command).unwrap_or(providers::CLAUDE_INSTANCE.to_string());
    let model = translate_model(&agents, &instance, command)?;

    ctx.sink
        .create_thread(
            viewer,
            NewThread {
                task_id: thread_id(command)?.to_string(),
                workspace_id: command
                    .get("projectId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| missing("projectId"))?
                    .to_string(),
                instance: instance.clone(),
                title: command
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                model,
                first_message: None,
            },
        )
        .await
}

/// Title and model. `branch`, `expectedBranch` and `worktreePath` are read and
/// ignored: a Task's branch is decided at creation and the checkout lives in the
/// Sandbox.
async fn meta_update(
    ctx: &CommandContext<'_>,
    viewer: &ScopeContext,
    command: &Value,
) -> Result<(), CommandRefusal> {
    let task_id = thread_id(command)?.to_string();
    let conversation = require_writable(ctx, viewer, &task_id).await?;
    let agents = ctx.agents.list(viewer).await;

    // A meta update carrying the other instance's model is not worth refusing:
    // the client sends one before every turn, and refusing here would abort the
    // send before the turn itself could explain why.
    let model = model_for_existing(&agents, &conversation, command).unwrap_or(None);

    ctx.sink
        .update_meta(
            viewer,
            ThreadMeta {
                task_id,
                title: command
                    .get("title")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                model,
            },
        )
        .await
}

async fn respond(
    ctx: &CommandContext<'_>,
    viewer: &ScopeContext,
    command: &Value,
) -> Result<(), CommandRefusal> {
    let task_id = thread_id(command)?.to_string();
    ctx.sink
        .answer_question(
            viewer,
            QuestionAnswer {
                task_id,
                tool_use_id: command
                    .get("requestId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| missing("requestId"))?
                    .to_string(),
                tool_name: "AskUserQuestion".to_string(),
                payload: command.get("answers").cloned(),
            },
        )
        .await
}

/// The Task, if this viewer may write to it.
///
/// Resolved through the same narrowed listing the read path uses, so the follow
/// rule, the read rule and the open-status rule all apply to a write exactly as
/// they apply to a read. A Task that fails any of them is reported the same way
/// as one that does not exist.
async fn require_writable(
    ctx: &CommandContext<'_>,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Conversation, CommandRefusal> {
    ctx.conversations
        .list_followed_by_ids(viewer, std::slice::from_ref(&task_id.to_string()))
        .await
        .map_err(|error| {
            tracing::warn!(%error, task_id, "T3 command could not resolve its thread");
            CommandRefusal::Failed {
                message: "Could not reach that thread.".to_string(),
            }
        })?
        .into_iter()
        .next()
        .ok_or_else(|| CommandRefusal::NotWritable {
            task_id: task_id.to_string(),
        })
}

/// The catalog model a command asks for, checked against the Task's own agent.
///
/// A Task's harness is fixed at creation — there is no column for it in
/// `TaskPatch` and no path that changes it — so a model belonging to the other
/// instance cannot be honoured. Refusing is the only honest answer: running it
/// on the thread's own agent instead is the silent substitution the published
/// model names exist to prevent.
fn model_for_existing(
    agents: &[crate::agent::schemas::AgentDescriptor],
    conversation: &Conversation,
    command: &Value,
) -> Result<Option<String>, CommandRefusal> {
    let Some(instance) = selected_instance(command) else {
        return Ok(None);
    };
    let thread_instance = catalog::find(conversation.agent_type.as_str())
        .map_or(providers::CLAUDE_INSTANCE, |entry| {
            providers::instance_for(entry.kind)
        });
    if instance != thread_instance {
        return Err(CommandRefusal::Rejected {
            message: format!(
                "This thread runs a {thread_instance} agent and cannot switch to {instance}. \
                 Start a new thread to use a different agent."
            ),
        });
    }
    translate_model(agents, &instance, command)
}

/// The catalog id behind the model slug on a command, if it carries one.
fn translate_model(
    agents: &[crate::agent::schemas::AgentDescriptor],
    instance: &str,
    command: &Value,
) -> Result<Option<String>, CommandRefusal> {
    let Some(slug) = command
        .get("modelSelection")
        .and_then(|m| m.get("model"))
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    providers::catalog_model_for(agents, instance, slug)
        .map(Some)
        .ok_or_else(|| CommandRefusal::Rejected {
            message: format!("`{slug}` is not a model this server offers."),
        })
}

fn selected_instance(command: &Value) -> Option<String> {
    command
        .get("modelSelection")
        .and_then(|m| m.get("instanceId"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// T3's plan/default toggle, as a Moatless agent mode.
fn interaction_mode(command: &Value) -> Option<AgentMode> {
    match command.get("interactionMode").and_then(Value::as_str) {
        Some("plan") => Some(AgentMode("plan".to_string())),
        Some("default") => Some(AgentMode("edit".to_string())),
        _ => None,
    }
}

fn thread_id(command: &Value) -> Result<&str, CommandRefusal> {
    command
        .get("threadId")
        .and_then(Value::as_str)
        .ok_or_else(|| missing("threadId"))
}

fn missing(field: &str) -> CommandRefusal {
    CommandRefusal::Rejected {
        message: format!("The command is missing `{field}`."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared_models::AgentType;
    use crate::task_listing::models::{EnvironmentIdentity, ListingHead};
    use crate::ui_rpc::dao::MockConversationDao;
    use async_trait::async_trait;
    use std::sync::Mutex;

    /// What the sink was asked to do. Every assertion about the command table
    /// is an assertion about this list, because the table's whole job is
    /// deciding which of these happens.
    #[derive(Debug, PartialEq, Eq)]
    enum Call {
        SendTurn {
            task_id: String,
            message_uuid: String,
            model: Option<String>,
            agent_mode: Option<String>,
        },
        CreateThread {
            task_id: String,
            workspace_id: String,
            instance: String,
            has_first_message: bool,
        },
        UpdateMeta {
            task_id: String,
            title: Option<String>,
            model: Option<String>,
        },
        Stop {
            task_id: String,
        },
        AnswerQuestion {
            task_id: String,
            tool_use_id: String,
        },
    }

    #[derive(Default)]
    struct RecordingSink(Mutex<Vec<Call>>);

    #[async_trait]
    impl ThreadCommandSink for RecordingSink {
        async fn send_turn(
            &self,
            _viewer: &ScopeContext,
            turn: ThreadTurn,
        ) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::SendTurn {
                task_id: turn.task_id,
                message_uuid: turn.message_uuid,
                model: turn.model,
                agent_mode: turn.agent_mode.map(|m| m.0),
            });
            Ok(())
        }
        async fn create_thread(
            &self,
            _viewer: &ScopeContext,
            thread: NewThread,
        ) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::CreateThread {
                task_id: thread.task_id,
                workspace_id: thread.workspace_id,
                instance: thread.instance,
                has_first_message: thread.first_message.is_some(),
            });
            Ok(())
        }
        async fn update_meta(
            &self,
            _viewer: &ScopeContext,
            meta: ThreadMeta,
        ) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::UpdateMeta {
                task_id: meta.task_id,
                title: meta.title,
                model: meta.model,
            });
            Ok(())
        }
        async fn stop(&self, _viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::Stop {
                task_id: task_id.to_string(),
            });
            Ok(())
        }
        async fn answer_question(
            &self,
            _viewer: &ScopeContext,
            answer: QuestionAnswer,
        ) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::AnswerQuestion {
                task_id: answer.task_id,
                tool_use_id: answer.tool_use_id,
            });
            Ok(())
        }
    }

    struct StubAgents;

    #[async_trait]
    impl AgentCatalogSource for StubAgents {
        async fn list(
            &self,
            _viewer: &ScopeContext,
        ) -> Vec<crate::agent::schemas::AgentDescriptor> {
            catalog::catalog()
                .iter()
                .map(|entry| crate::agent::schemas::AgentDescriptor {
                    id: entry.id.to_string(),
                    label: entry.label.to_string(),
                    installed: true,
                    default_model: entry.default_model.to_string(),
                    models: entry
                        .models
                        .iter()
                        .map(|m| crate::agent::schemas::AgentModel {
                            id: m.id.to_string(),
                            label: m.label.to_string(),
                        })
                        .collect(),
                    missing_credential_hint: None,
                })
                .collect()
        }
    }

    fn conversation(agent_type: &str) -> Conversation {
        Conversation {
            task_id: "task-1".to_string(),
            workspace_id: Some("ws-1".to_string()),
            name: None,
            branch: None,
            agent_type: AgentType(agent_type.to_string()),
            created_at: "2026-07-01T09:00:00.000Z".to_string(),
            created_at_db: None,
            updated_at: None,
            turn_number: None,
            turn_status: None,
            turn_started_at: None,
            turn_ended_at: None,
            sandbox_desired_state: None,
            sandbox_stop_reason: None,
            sandbox_started_at: None,
            sandbox_updated_at: None,
            latest_user_message_at: None,
            assistant_model: None,
            requested_model: None,
            pending_question: false,
            pending_plan: false,
        }
    }

    /// A DAO that answers with one conversation, or with none when `found` is
    /// false — which is what "the viewer cannot write to it" looks like from
    /// here.
    fn conversations(found: Option<Conversation>) -> MockConversationDao {
        let mut dao = MockConversationDao::new();
        dao.expect_list_followed_by_ids().returning(move |_, _| {
            Ok(found.clone().into_iter().collect::<Vec<_>>())
        });
        dao.expect_listing_head().returning(|| {
            Ok(ListingHead {
                identity: EnvironmentIdentity::new("test"),
                position: 42,
            })
        });
        dao
    }

    fn viewer() -> ScopeContext {
        ScopeContext {
            user_id: "user-1".to_string(),
            is_admin: false,
        }
    }

    /// Run one command against a Claude thread that exists.
    async fn run(command: Value) -> (Result<Value, CommandRefusal>, Vec<Call>) {
        run_against(command, Some(conversation(AgentType::CLAUDE_CODE))).await
    }

    async fn run_against(
        command: Value,
        found: Option<Conversation>,
    ) -> (Result<Value, CommandRefusal>, Vec<Call>) {
        let dao = conversations(found);
        let sink = RecordingSink::default();
        let agents = StubAgents;
        let ctx = CommandContext {
            conversations: &dao,
            agents: &agents,
            sink: &sink,
        };
        let result = dispatch(&ctx, &viewer(), &command).await;
        let calls = std::mem::take(&mut *sink.0.lock().unwrap());
        (result, calls)
    }

    fn turn(model: Option<(&str, &str)>) -> Value {
        let mut command = json!({
            "type": "thread.turn.start",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "message": {
                "messageId": "msg-1",
                "role": "user",
                "text": "have another look",
                "attachments": []
            },
            "interactionMode": "default",
            "createdAt": "2026-07-01T09:00:00.000Z"
        });
        if let Some((instance, slug)) = model {
            command["modelSelection"] = json!({ "instanceId": instance, "model": slug });
        }
        command
    }

    #[tokio::test]
    async fn a_turn_persists_under_the_id_the_client_minted() {
        let (result, calls) = run(turn(None)).await;
        assert!(result.is_ok());
        assert_eq!(
            calls,
            vec![Call::SendTurn {
                task_id: "task-1".to_string(),
                message_uuid: "msg-1".to_string(),
                model: None,
                agent_mode: Some("edit".to_string()),
            }]
        );
    }

    #[tokio::test]
    async fn a_published_slug_reaches_the_sink_as_a_catalog_id() {
        let (_, calls) = run(turn(Some(("claudeAgent", "claude-sonnet-5")))).await;
        assert_eq!(
            calls,
            vec![Call::SendTurn {
                task_id: "task-1".to_string(),
                message_uuid: "msg-1".to_string(),
                model: Some("sonnet".to_string()),
                agent_mode: Some("edit".to_string()),
            }]
        );
    }

    /// A task's harness is fixed at creation, so this cannot be honoured. The
    /// point of the assertion is that nothing reaches the sink: refusing after
    /// a write would leave the message in a conversation the person was told
    /// had refused it.
    #[tokio::test]
    async fn a_model_from_the_other_instance_is_refused_before_any_write() {
        let (result, calls) = run(turn(Some(("codex", "gpt-5.6-sol")))).await;
        assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
        assert!(calls.is_empty());
    }

    /// Silently substituting the default is the failure the published model
    /// names exist to prevent, so an unknown slug refuses rather than falls
    /// back.
    #[tokio::test]
    async fn a_model_we_never_published_is_refused_rather_than_defaulted() {
        let (result, calls) = run(turn(Some(("claudeAgent", "claude-opus-4-8")))).await;
        assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
        assert!(calls.is_empty());
    }

    #[tokio::test]
    async fn plan_mode_reaches_the_sink_as_the_moatless_plan_mode() {
        let mut command = turn(None);
        command["interactionMode"] = json!("plan");
        let (_, calls) = run(command).await;
        assert!(matches!(
            &calls[0],
            Call::SendTurn { agent_mode, .. } if agent_mode.as_deref() == Some("plan")
        ));
    }

    #[tokio::test]
    async fn a_thread_the_viewer_cannot_write_to_is_refused() {
        let (result, calls) = run_against(turn(None), None).await;
        assert!(matches!(result, Err(CommandRefusal::NotWritable { .. })));
        assert!(calls.is_empty());
    }

    #[tokio::test]
    async fn a_bootstrapped_turn_creates_the_thread_with_its_first_message() {
        let mut command = turn(Some(("claudeAgent", "claude-opus-5")));
        command["bootstrap"] = json!({
            "createThread": {
                "projectId": "ws-1",
                "title": "New chat",
                "runtimeMode": "full-access",
                "interactionMode": "default",
                "branch": null,
                "worktreePath": null,
                "createdAt": "2026-07-01T09:00:00.000Z"
            }
        });
        // A bootstrapped turn never resolves an existing thread — there is not
        // one yet.
        let (result, calls) = run_against(command, None).await;
        assert!(result.is_ok());
        assert_eq!(
            calls,
            vec![Call::CreateThread {
                task_id: "task-1".to_string(),
                workspace_id: "ws-1".to_string(),
                instance: "claudeAgent".to_string(),
                has_first_message: true,
            }]
        );
    }

    #[tokio::test]
    async fn a_standalone_create_carries_no_first_message() {
        let command = json!({
            "type": "thread.create",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "projectId": "ws-1",
            "title": "New chat",
            "modelSelection": { "instanceId": "codex", "model": "gpt-5.6-sol" },
            "runtimeMode": "full-access",
            "interactionMode": "default",
            "branch": null,
            "worktreePath": null,
            "createdAt": "2026-07-01T09:00:00.000Z"
        });
        let (result, calls) = run_against(command, None).await;
        assert!(result.is_ok());
        assert_eq!(
            calls,
            vec![Call::CreateThread {
                task_id: "task-1".to_string(),
                workspace_id: "ws-1".to_string(),
                instance: "codex".to_string(),
                has_first_message: false,
            }]
        );
    }

    #[tokio::test]
    async fn a_meta_update_carries_title_and_translated_model() {
        let command = json!({
            "type": "thread.meta.update",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "title": "Fix the checkout",
            "modelSelection": { "instanceId": "claudeAgent", "model": "claude-haiku-4-5" },
            "branch": "feat/checkout",
            "worktreePath": null
        });
        let (result, calls) = run(command).await;
        assert!(result.is_ok());
        assert_eq!(
            calls,
            vec![Call::UpdateMeta {
                task_id: "task-1".to_string(),
                title: Some("Fix the checkout".to_string()),
                model: Some("haiku".to_string()),
            }]
        );
    }

    /// The client sends a meta update before every turn. Refusing one because
    /// it names the other instance's model would abort the send before the turn
    /// itself could explain why.
    #[tokio::test]
    async fn a_meta_update_naming_the_other_instance_still_updates_the_title() {
        let command = json!({
            "type": "thread.meta.update",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "title": "Fix the checkout",
            "modelSelection": { "instanceId": "codex", "model": "gpt-5.6-sol" }
        });
        let (result, calls) = run(command).await;
        assert!(result.is_ok());
        assert_eq!(
            calls,
            vec![Call::UpdateMeta {
                task_id: "task-1".to_string(),
                title: Some("Fix the checkout".to_string()),
                model: None,
            }]
        );
    }

    /// Moatless draws no line between interrupting a turn and stopping a
    /// session, so both arrive at the same stop.
    #[tokio::test]
    async fn interrupt_and_session_stop_both_stop_the_agent() {
        for tag in ["thread.turn.interrupt", "thread.session.stop"] {
            let command = json!({
                "type": tag,
                "commandId": "cmd-1",
                "threadId": "task-1",
                "createdAt": "2026-07-01T09:00:00.000Z"
            });
            let (result, calls) = run(command).await;
            assert!(result.is_ok(), "{tag}");
            assert_eq!(
                calls,
                vec![Call::Stop {
                    task_id: "task-1".to_string()
                }],
                "{tag}"
            );
        }
    }

    #[tokio::test]
    async fn an_answer_reaches_the_agent_as_a_tool_response() {
        let command = json!({
            "type": "thread.user-input.respond",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "requestId": "toolu_1",
            "answers": { "choice": "yes" },
            "createdAt": "2026-07-01T09:00:00.000Z"
        });
        let (result, calls) = run(command).await;
        assert!(result.is_ok());
        assert_eq!(
            calls,
            vec![Call::AnswerQuestion {
                task_id: "task-1".to_string(),
                tool_use_id: "toolu_1".to_string(),
            }]
        );
    }

    /// Both controls exist on screen and neither has a Moatless meaning.
    /// Refusing would put an error behind a control that does nothing.
    #[tokio::test]
    async fn the_two_mode_commands_are_accepted_and_do_nothing() {
        for command in [
            json!({
                "type": "thread.runtime-mode.set",
                "commandId": "cmd-1",
                "threadId": "task-1",
                "runtimeMode": "full-access",
                "createdAt": "2026-07-01T09:00:00.000Z"
            }),
            json!({
                "type": "thread.interaction-mode.set",
                "commandId": "cmd-1",
                "threadId": "task-1",
                "interactionMode": "plan",
                "createdAt": "2026-07-01T09:00:00.000Z"
            }),
        ] {
            let tag = command["type"].as_str().unwrap().to_string();
            let (result, calls) = run(command).await;
            assert!(result.is_ok(), "{tag}");
            assert!(calls.is_empty(), "{tag}");
        }
    }

    #[tokio::test]
    async fn every_unsupported_command_is_refused_without_a_write() {
        for tag in [
            "thread.delete",
            "thread.archive",
            "thread.unarchive",
            "project.create",
            "project.meta.update",
            "project.delete",
            "thread.approval.respond",
            "thread.checkpoint.revert",
            "thread.settle",
            "thread.unsettle",
            "thread.snooze",
            "thread.unsnooze",
            "thread.something.invented",
        ] {
            let command = json!({
                "type": tag,
                "commandId": "cmd-1",
                "threadId": "task-1"
            });
            let (result, calls) = run(command).await;
            assert!(
                matches!(result, Err(CommandRefusal::Unsupported { .. })),
                "{tag} was not refused"
            );
            assert!(calls.is_empty(), "{tag} reached the sink");
        }
    }

    #[tokio::test]
    async fn a_successful_command_reports_the_listing_position() {
        let (result, _) = run(turn(None)).await;
        assert_eq!(result.unwrap(), json!({ "sequence": 42 }));
    }
}
