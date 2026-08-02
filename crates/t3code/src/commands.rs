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
//! has not been written yet: Moatless has no delete that leaves the Task
//! recoverable, Workspaces are not authored from here, and approvals and
//! checkpoints are never projected in the first place, so no control raises
//! them.
//!
//! Archiving is the one command whose two vocabularies differ: T3 archives a
//! thread, Moatless closes a Task, and they are the same act — see
//! [`ThreadCommandSink::archive`](super::ports::ThreadCommandSink::archive).

use chrono::{DateTime, Utc};
use serde_json::{Value, json};

use moatless_backend_core::scope::ScopeContext;

use super::dao::{Conversation, ConversationDao};
use super::ports::{
    AgentCatalogSource, CommandRefusal, NewThread, QuestionAnswer, ThreadCommandSink, ThreadMeta,
    ThreadTurn,
};
use super::providers;
use crate::agent::catalog;
use moatless_api_schemas::shared::AgentMode;
use moatless_backend_core::activity::mapping;

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
        "thread.archive" => {
            ctx.sink.archive(viewer, thread_id(command)?).await?;
        }
        "thread.unarchive" => {
            ctx.sink.unarchive(viewer, thread_id(command)?).await?;
        }
        "thread.settle" => {
            ctx.sink.settle(viewer, thread_id(command)?).await?;
        }
        "thread.unsettle" => {
            ctx.sink.unsettle(viewer, thread_id(command)?).await?;
        }
        "thread.snooze" => {
            let until = command
                .get("snoozedUntil")
                .and_then(Value::as_str)
                .ok_or_else(|| missing("snoozedUntil"))?;
            let until = DateTime::parse_from_rfc3339(until)
                .map_err(|_| CommandRefusal::Rejected {
                    message: format!("`{until}` is not a timestamp."),
                })?
                .with_timezone(&Utc);
            if until <= Utc::now() {
                return Err(CommandRefusal::Rejected {
                    message: "A snooze must wake in the future.".to_string(),
                });
            }
            ctx.sink.snooze(viewer, thread_id(command)?, until).await?;
        }
        "thread.unsnooze" => {
            ctx.sink.unsnooze(viewer, thread_id(command)?).await?;
        }
        "thread.user-input.respond" => respond(ctx, viewer, command).await?,

        // Accepted and discarded — see the module comment.
        "thread.runtime-mode.set" | "thread.interaction-mode.set" => {}

        "thread.delete" => {
            return Err(CommandRefusal::Unsupported {
                command: tag,
                reason: "Archive it instead, or delete it from the Moatless UI.".to_string(),
            });
        }
        "project.create" | "project.meta.update" | "project.delete" => {
            return Err(CommandRefusal::Unsupported {
                command: tag,
                reason: "Projects are Moatless workspaces and are managed there.".to_string(),
            });
        }
        "thread.approval.respond" | "thread.checkpoint.revert" => {
            return Err(CommandRefusal::Unsupported {
                command: tag,
                reason: "Approvals and checkpoints are not part of this server.".to_string(),
            });
        }
        other => {
            return Err(CommandRefusal::Unsupported {
                command: other.to_string(),
                reason: "This server does not implement that action.".to_string(),
            });
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
    let effort = selected_effort(command);

    let bootstrap = command.get("bootstrap").and_then(|b| b.get("createThread"));

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
                    branch: branch(create),
                    first_message: Some(ThreadTurn {
                        task_id,
                        text,
                        message_uuid,
                        model,
                        agent_mode,
                        effort,
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
                effort,
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
                branch: branch(command),
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

/// An answer to what the agent asked.
///
/// `requestId` is the tool-use id of the call that asked — the panel is derived
/// from an activity whose `requestId` is exactly that — so the stored call is
/// what says which tool this answers, and therefore what an answer to it looks
/// like. Two tools ask, and they take different answers: a question takes one
/// per question, a proposed plan takes a yes or a no. Assuming either would
/// label a plan answer as a question answer and send a rejection as a blob.
async fn respond(
    ctx: &CommandContext<'_>,
    viewer: &ScopeContext,
    command: &Value,
) -> Result<(), CommandRefusal> {
    let task_id = thread_id(command)?.to_string();
    let tool_use_id = command
        .get("requestId")
        .and_then(Value::as_str)
        .ok_or_else(|| missing("requestId"))?
        .to_string();

    let call = ctx
        .conversations
        .interactive_call(&task_id, &tool_use_id)
        .await
        .map_err(|error| {
            tracing::warn!(%error, task_id, tool_use_id, "T3 answer could not resolve its call");
            CommandRefusal::Failed {
                message: "Could not reach that thread.".to_string(),
            }
        })?
        .ok_or_else(|| CommandRefusal::Rejected {
            message: "That request is not one this thread is waiting on.".to_string(),
        })?;
    if call.settled {
        // Answered already — by someone else, or from the Moatless UI. Saying so
        // is the honest report; a send failure would be a different fact.
        return Err(CommandRefusal::Rejected {
            message: "That request has already been answered.".to_string(),
        });
    }

    let answers = answers_by_question(command.get("answers"));
    let payload = match call.tool_name.as_str() {
        mapping::QUESTION_TOOL => json!({ "answers": answers }),
        mapping::PLAN_TOOL => plan_verdict(&answers),
        other => {
            return Err(CommandRefusal::Rejected {
                message: format!("`{other}` is not a request this server can be answered on."),
            });
        }
    };

    ctx.sink
        .answer_question(
            viewer,
            QuestionAnswer {
                task_id,
                tool_use_id,
                tool_name: call.tool_name,
                payload: Some(payload),
            },
        )
        .await
}

/// One answer per question, keyed on the question's full text.
///
/// The client accumulates `string | string[]` per question and a multi-select
/// answer is joined here, on the way in: the CLI merges the value into the tool
/// input unread, and Moatless's own panel already sends the joined string, so a
/// second answer shape would buy nothing. Anything that is neither is dropped
/// rather than stringified into JSON the agent would have to parse.
fn answers_by_question(answers: Option<&Value>) -> Value {
    let mut by_question = serde_json::Map::new();
    for (question, answer) in answers.and_then(Value::as_object).into_iter().flatten() {
        let answer = match answer {
            Value::String(text) => text.clone(),
            Value::Array(parts) => parts
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", "),
            _ => continue,
        };
        by_question.insert(question.clone(), Value::String(answer));
    }
    Value::Object(by_question)
}

/// A proposed plan answered through the question in front of it.
///
/// Only the approve option approves. Everything else is a rejection carrying
/// what was said as feedback — which is what makes "No, use the second
/// approach" reach the agent as a rejection *and* as the reason.
fn plan_verdict(answers: &Value) -> Value {
    let said = answers
        .get(mapping::PLAN_QUESTION)
        .and_then(Value::as_str)
        .or_else(|| {
            answers
                .as_object()
                .and_then(|answers| answers.values().next())
                .and_then(Value::as_str)
        })
        .unwrap_or_default();
    if said == mapping::PLAN_APPROVE {
        return json!({ "approved": true });
    }
    json!({ "approved": false, "feedback": said })
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

/// The branch a create command carries, if it names a non-empty one.
///
/// The contract sends `null` for "the repository's default", which reaches
/// creation as `None` and resolves to the default there. A blank string is the
/// same intent and is read the same way rather than pinned as a branch named
/// "".
fn branch(command: &Value) -> Option<String> {
    command
        .get("branch")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|branch| !branch.is_empty())
        .map(str::to_string)
}

fn selected_instance(command: &Value) -> Option<String> {
    command
        .get("modelSelection")
        .and_then(|m| m.get("instanceId"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// The reasoning effort a command selected, in the agent's own vocabulary.
///
/// T3 carries every per-model choice as one array of `{id, value}` selections
/// on `modelSelection.options`, keyed by the descriptor ids the provider
/// published (`providers::effort_option_id`). Only the effort descriptor is read
/// here; a selection this server never published is another provider's and is
/// ignored rather than forwarded to an agent that has no such setting.
fn selected_effort(command: &Value) -> Option<String> {
    let instance = selected_instance(command)?;
    let wanted = providers::effort_option_id(&instance);
    command
        .get("modelSelection")
        .and_then(|m| m.get("options"))
        .and_then(Value::as_array)?
        .iter()
        .find(|option| option.get("id").and_then(Value::as_str) == Some(wanted))
        .and_then(|option| option.get("value"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
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
    use crate::dao::{InteractiveCall, MockConversationDao};
    use crate::listing::{UiRpcEnvironmentIdentity, UiRpcListingHead};
    use async_trait::async_trait;
    use moatless_api_schemas::shared::AgentType;
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
            effort: Option<String>,
        },
        CreateThread {
            task_id: String,
            workspace_id: String,
            instance: String,
            branch: Option<String>,
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
        Archive {
            task_id: String,
        },
        Unarchive {
            task_id: String,
        },
        Settle {
            task_id: String,
        },
        Unsettle {
            task_id: String,
        },
        Snooze {
            task_id: String,
            snoozed_until: String,
        },
        Unsnooze {
            task_id: String,
        },
        AnswerQuestion {
            task_id: String,
            tool_use_id: String,
            tool_name: String,
            payload: Option<Value>,
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
                effort: turn.effort,
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
                branch: thread.branch,
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
        async fn archive(
            &self,
            _viewer: &ScopeContext,
            task_id: &str,
        ) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::Archive {
                task_id: task_id.to_string(),
            });
            Ok(())
        }
        async fn unarchive(
            &self,
            _viewer: &ScopeContext,
            task_id: &str,
        ) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::Unarchive {
                task_id: task_id.to_string(),
            });
            Ok(())
        }
        async fn settle(
            &self,
            _viewer: &ScopeContext,
            task_id: &str,
        ) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::Settle {
                task_id: task_id.to_string(),
            });
            Ok(())
        }
        async fn unsettle(
            &self,
            _viewer: &ScopeContext,
            task_id: &str,
        ) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::Unsettle {
                task_id: task_id.to_string(),
            });
            Ok(())
        }
        async fn snooze(
            &self,
            _viewer: &ScopeContext,
            task_id: &str,
            snoozed_until: DateTime<Utc>,
        ) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::Snooze {
                task_id: task_id.to_string(),
                snoozed_until: snoozed_until.to_rfc3339(),
            });
            Ok(())
        }
        async fn unsnooze(
            &self,
            _viewer: &ScopeContext,
            task_id: &str,
        ) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::Unsnooze {
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
                tool_name: answer.tool_name,
                payload: answer.payload,
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
            closed_at: None,
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
            settled_override: None,
            settled_at: None,
            snoozed_until: None,
            snoozed_at: None,
        }
    }

    /// A DAO that answers with one conversation, or with none when `found` is
    /// false — which is what "the viewer cannot write to it" looks like from
    /// here.
    fn conversations(found: Option<Conversation>) -> MockConversationDao {
        let mut dao = MockConversationDao::new();
        dao.expect_list_followed_by_ids()
            .returning(move |_, _| Ok(found.clone().into_iter().collect::<Vec<_>>()));
        dao.expect_listing_head().returning(|| {
            Ok(UiRpcListingHead {
                identity: UiRpcEnvironmentIdentity::new("test"),
                position: 42,
            })
        });
        dao
    }

    /// The stored call an answer names — the unanswered `AskUserQuestion` a
    /// panel is derived from, unless a test says otherwise.
    fn asked(tool_name: &str) -> InteractiveCall {
        InteractiveCall {
            tool_name: tool_name.to_string(),
            settled: false,
        }
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

    /// Run one command against a thread whose stored call is `call`.
    async fn run_answering(
        command: Value,
        call: Option<InteractiveCall>,
    ) -> (Result<Value, CommandRefusal>, Vec<Call>) {
        let mut dao = conversations(Some(conversation(AgentType::CLAUDE_CODE)));
        dao.expect_interactive_call()
            .returning(move |_, _| Ok(call.clone()));
        dispatch_against(dao, command).await
    }

    async fn run_against(
        command: Value,
        found: Option<Conversation>,
    ) -> (Result<Value, CommandRefusal>, Vec<Call>) {
        let mut dao = conversations(found);
        dao.expect_interactive_call()
            .returning(|_, _| Ok(Some(asked(mapping::QUESTION_TOOL))));
        dispatch_against(dao, command).await
    }

    async fn dispatch_against(
        dao: MockConversationDao,
        command: Value,
    ) -> (Result<Value, CommandRefusal>, Vec<Call>) {
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

    /// The client's answer to whatever the thread is waiting on.
    fn answer(answers: Value) -> Value {
        json!({
            "type": "thread.user-input.respond",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "requestId": "toolu_1",
            "answers": answers,
            "createdAt": "2026-07-01T09:00:00.000Z"
        })
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
                effort: None,
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
                effort: None,
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

    /// The option array the client sends back, keyed by the descriptor ids
    /// `providers` published.
    fn turn_with_options(instance: &str, slug: &str, options: Value) -> Value {
        let mut command = turn(Some((instance, slug)));
        command["modelSelection"]["options"] = options;
        command
    }

    fn sent_effort(calls: &[Call]) -> Option<String> {
        match calls.first() {
            Some(Call::SendTurn { effort, .. }) => effort.clone(),
            other => panic!("expected a SendTurn, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_selected_effort_reaches_the_sink() {
        let command = turn_with_options(
            "claudeAgent",
            "claude-sonnet-5",
            json!([{ "id": "effort", "value": "xhigh" }]),
        );
        let (_, calls) = run(command).await;
        assert_eq!(sent_effort(&calls).as_deref(), Some("xhigh"));
    }

    #[tokio::test]
    async fn codex_reads_its_effort_off_its_own_descriptor_id() {
        let command = turn_with_options(
            "codex",
            "gpt-5.6-sol",
            json!([{ "id": "reasoningEffort", "value": "minimal" }]),
        );
        let (_, calls) = run_against(command, Some(conversation(AgentType::CODEX))).await;
        assert_eq!(sent_effort(&calls).as_deref(), Some("minimal"));
    }

    /// A selection keyed by an id this server never published for the instance
    /// belongs to some other provider; forwarding it would hand an agent a
    /// setting it has no vocabulary for.
    #[tokio::test]
    async fn an_option_the_instance_never_published_is_ignored() {
        let command = turn_with_options(
            "claudeAgent",
            "claude-sonnet-5",
            json!([{ "id": "reasoningEffort", "value": "minimal" }, { "id": "verbosity", "value": "high" }]),
        );
        let (_, calls) = run(command).await;
        assert_eq!(sent_effort(&calls), None);
    }

    #[tokio::test]
    async fn a_turn_that_picked_no_effort_leaves_the_agent_on_its_default() {
        let (_, calls) = run(turn(Some(("claudeAgent", "claude-sonnet-5")))).await;
        assert_eq!(sent_effort(&calls), None);
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
                branch: None,
                has_first_message: true,
            }]
        );
    }

    /// The branch the person picked in the composer reaches the sink, so the
    /// Task is created against it rather than the repository's default.
    #[tokio::test]
    async fn a_bootstrapped_turn_carries_the_picked_branch() {
        let mut command = turn(Some(("claudeAgent", "claude-opus-5")));
        command["bootstrap"] = json!({
            "createThread": {
                "projectId": "ws-1",
                "title": "New chat",
                "runtimeMode": "full-access",
                "interactionMode": "default",
                "branch": "feat/checkout",
                "worktreePath": null,
                "createdAt": "2026-07-01T09:00:00.000Z"
            }
        });
        let (result, calls) = run_against(command, None).await;
        assert!(result.is_ok());
        assert!(matches!(
            calls.as_slice(),
            [Call::CreateThread { branch, .. }] if branch.as_deref() == Some("feat/checkout")
        ));
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
                branch: None,
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
    async fn settle_commands_reach_the_sink() {
        for (command, expected) in [
            (
                json!({
                    "type": "thread.settle",
                    "commandId": "cmd-1",
                    "threadId": "task-1",
                }),
                Call::Settle {
                    task_id: "task-1".to_string(),
                },
            ),
            (
                json!({
                    "type": "thread.unsettle",
                    "commandId": "cmd-1",
                    "threadId": "task-1",
                }),
                Call::Unsettle {
                    task_id: "task-1".to_string(),
                },
            ),
            (
                json!({
                    "type": "thread.unsnooze",
                    "commandId": "cmd-1",
                    "threadId": "task-1",
                }),
                Call::Unsnooze {
                    task_id: "task-1".to_string(),
                },
            ),
        ] {
            let tag = command["type"].as_str().unwrap().to_string();
            let (result, calls) = run(command).await;
            assert!(result.is_ok(), "{tag}");
            assert_eq!(calls, vec![expected], "{tag}");
        }
    }

    #[tokio::test]
    async fn snooze_reaches_the_sink_with_its_wake_time() {
        let command = json!({
            "type": "thread.snooze",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "snoozedUntil": "2099-07-01T09:00:00.000Z",
        });
        let (result, calls) = run(command).await;
        assert!(result.is_ok());
        assert_eq!(
            calls,
            vec![Call::Snooze {
                task_id: "task-1".to_string(),
                snoozed_until: "2099-07-01T09:00:00+00:00".to_string(),
            }]
        );
    }

    #[tokio::test]
    async fn snooze_rejects_a_missing_wake_time_before_any_write() {
        let command = json!({
            "type": "thread.snooze",
            "commandId": "cmd-1",
            "threadId": "task-1",
        });
        let (result, calls) = run(command).await;
        assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
        assert!(calls.is_empty());
    }

    #[tokio::test]
    async fn snooze_rejects_an_unparseable_wake_time_before_any_write() {
        let command = json!({
            "type": "thread.snooze",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "snoozedUntil": "tomorrow",
        });
        let (result, calls) = run(command).await;
        assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
        assert!(calls.is_empty());
    }

    #[tokio::test]
    async fn snooze_rejects_a_past_wake_time_before_any_write() {
        let command = json!({
            "type": "thread.snooze",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "snoozedUntil": "2020-07-01T09:00:00.000Z",
        });
        let (result, calls) = run(command).await;
        assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
        assert!(calls.is_empty());
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
                tool_name: mapping::QUESTION_TOOL.to_string(),
                payload: Some(json!({ "answers": { "choice": "yes" } })),
            }]
        );
    }

    /// A plan is answered through the question joined to it, so the same
    /// command has to become a verdict rather than a blob of answers. Only the
    /// approve option approves; anything else is a rejection carrying what was
    /// said, which is what makes "No, use the second approach" reach the agent
    /// as a rejection *and* as the reason.
    #[tokio::test]
    async fn a_plan_is_answered_as_a_verdict() {
        for (said, expected) in [
            (mapping::PLAN_APPROVE, json!({ "approved": true })),
            (
                "Reject: use the second approach",
                json!({ "approved": false, "feedback": "Reject: use the second approach" }),
            ),
        ] {
            let (result, calls) = run_answering(
                answer(json!({ mapping::PLAN_QUESTION: said })),
                Some(asked(mapping::PLAN_TOOL)),
            )
            .await;
            assert!(result.is_ok(), "{said}");
            assert_eq!(
                calls,
                vec![Call::AnswerQuestion {
                    task_id: "task-1".to_string(),
                    tool_use_id: "toolu_1".to_string(),
                    tool_name: mapping::PLAN_TOOL.to_string(),
                    payload: Some(expected),
                }],
                "{said}"
            );
        }
    }

    /// A multi-select answer arrives as a list and the CLI merges the value into
    /// the tool input unread, so it is joined here rather than sent as an array
    /// the agent would have to parse.
    #[tokio::test]
    async fn a_multi_select_answer_reaches_the_agent_as_one_string() {
        let (_, calls) = run_answering(
            answer(json!({ "Which?": ["Postgres", "Redis"] })),
            Some(asked(mapping::QUESTION_TOOL)),
        )
        .await;
        let Some(Call::AnswerQuestion { payload, .. }) = calls.first() else {
            panic!("the answer reaches the sink: {calls:?}");
        };
        assert_eq!(
            payload.as_ref().unwrap()["answers"]["Which?"],
            "Postgres, Redis"
        );
    }

    /// An answer naming a call this thread never stored is not a failed send —
    /// there is nothing to send it to. The two read differently to the person
    /// who clicked.
    #[tokio::test]
    async fn an_answer_to_a_call_that_was_never_stored_is_refused() {
        let (result, calls) = run_answering(answer(json!({ "q": "yes" })), None).await;
        assert!(
            matches!(result, Err(CommandRefusal::Rejected { .. })),
            "{result:?}"
        );
        assert!(calls.is_empty());
    }

    /// The call came back already — from the Moatless UI, or from a second
    /// person. Sending a second answer would reach an agent that has moved on.
    #[tokio::test]
    async fn an_answer_to_a_settled_call_is_refused() {
        let (result, calls) = run_answering(
            answer(json!({ "q": "yes" })),
            Some(InteractiveCall {
                tool_name: mapping::QUESTION_TOOL.to_string(),
                settled: true,
            }),
        )
        .await;
        assert!(
            matches!(result, Err(CommandRefusal::Rejected { .. })),
            "{result:?}"
        );
        assert!(calls.is_empty());
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

    /// Archiving is closing and unarchiving is reopening, so both reach the
    /// sink rather than a refusal. This is the pair a person uses to put a
    /// finished conversation away and to pull it back out.
    #[tokio::test]
    async fn archive_and_unarchive_reach_the_sink() {
        for (tag, expected) in [
            (
                "thread.archive",
                Call::Archive {
                    task_id: "task-1".to_string(),
                },
            ),
            (
                "thread.unarchive",
                Call::Unarchive {
                    task_id: "task-1".to_string(),
                },
            ),
        ] {
            let (result, calls) = run(json!({
                "type": tag,
                "commandId": "cmd-1",
                "threadId": "task-1",
                "createdAt": "2026-07-01T09:00:00.000Z"
            }))
            .await;
            assert!(result.is_ok(), "{tag}: {result:?}");
            assert_eq!(calls, vec![expected], "{tag}");
        }
    }

    /// A command with no thread on it is rejected before anything is written.
    #[tokio::test]
    async fn archiving_without_a_thread_id_writes_nothing() {
        let (result, calls) = run(json!({ "type": "thread.archive", "commandId": "cmd-1" })).await;
        assert!(
            matches!(result, Err(CommandRefusal::Rejected { .. })),
            "{result:?}"
        );
        assert!(calls.is_empty());
    }

    #[tokio::test]
    async fn every_unsupported_command_is_refused_without_a_write() {
        for tag in [
            "thread.delete",
            "project.create",
            "project.meta.update",
            "project.delete",
            "thread.approval.respond",
            "thread.checkpoint.revert",
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
