//! `orchestration.dispatchCommand` — the one write the T3 client makes.
//!
//! T3's command union is wider than Moatless, so this module is the table
//! saying, per command, whether it is carried out, discarded or refused. State
//! all three explicitly: discarding and refusing look identical from here and
//! completely different to the person holding the mouse.
//!
//! Three are discarded on purpose: `thread.runtime-mode.set` (no Moatless
//! meaning), `thread.interaction-mode.set` (the Build/Plan choice rides the turn
//! itself), and `bootstrap.prepareWorktree` / `runSetupScript` (the Sandbox's
//! business). Everything else refused is refused because it cannot be honoured.
//!
//! Archiving is the one command whose vocabularies differ: T3 archives a thread,
//! Moatless closes a Task, and they are the same act. The two reverts differ the
//! other way: `thread.conversation.revert` cuts the transcript and is carried
//! out, while `thread.checkpoint.revert` also restores the files and is refused,
//! because nothing here snapshots a worktree.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tracing::warn;

use moatless_types::access_control::ScopeContext;

use base64::Engine;

use super::dao::{Conversation, ConversationDao};
use super::ports::{
    AgentCatalogSource, AttachmentKind, CommandRefusal, ForkThread, NewThread, QuestionAnswer,
    SkillCatalog, ThreadCommandSink, ThreadMeta, ThreadPullRequestRef, ThreadTurn, TurnAttachment,
};
use super::providers;
use crate::agent::catalog;
use crate::scripts::{self, ScriptsWriteError};
use crate::skills;
use moatless_types::activity as mapping;
use moatless_types::agent::AgentMode;
use moatless_types::task::Visibility;
use uuid::Uuid;

/// What the surface needs in order to answer a command.
pub struct CommandContext<'a> {
    pub conversations: &'a dyn ConversationDao,
    pub agents: &'a dyn AgentCatalogSource,
    pub sink: &'a dyn ThreadCommandSink,
    pub workspace_script_commands: &'a dyn crate::ports::WorkspaceScriptCommands,
    /// The skills of the Workspace a turn runs in, for resolving the `$name`
    /// mentions the composer leaves in the prompt.
    pub skills: &'a dyn SkillCatalog,
    /// The viewer, as the id plugin activation records are keyed by — see
    /// [`SkillCatalog::list_for_workspace`].
    pub caller: Uuid,
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
        "thread.fork" => fork_thread(ctx, viewer, command).await?,
        "thread.meta.update" => meta_update(ctx, viewer, command).await?,
        "thread.turn.interrupt" => {
            ctx.sink.interrupt(viewer, thread_id(command)?).await?;
        }
        "thread.session.stop" => {
            ctx.sink.stop(viewer, thread_id(command)?).await?;
        }
        "thread.archive" => {
            ctx.sink.archive(viewer, thread_id(command)?).await?;
        }
        "thread.unarchive" => {
            ctx.sink.unarchive(viewer, thread_id(command)?).await?;
        }
        "thread.follow" => {
            ctx.sink.follow(viewer, thread_id(command)?).await?;
        }
        "thread.unfollow" => {
            ctx.sink.unfollow(viewer, thread_id(command)?).await?;
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
        "thread.visibility.set" => {
            let visibility = command
                .get("visibility")
                .and_then(Value::as_str)
                .ok_or_else(|| missing("visibility"))?;
            let visibility =
                Visibility::try_from(visibility).map_err(|_| CommandRefusal::Rejected {
                    message: format!("`{visibility}` is not a visibility."),
                })?;
            ctx.sink
                .set_visibility(viewer, thread_id(command)?, visibility)
                .await?;
        }
        "thread.user-input.respond" => respond(ctx, viewer, command).await?,
        "thread.pull-request.link" => {
            let reference = pull_request_ref(ctx, viewer, command).await?;
            ctx.sink.link_pull_request(viewer, reference).await?;
        }
        "thread.pull-request.unlink" => {
            let reference = pull_request_ref(ctx, viewer, command).await?;
            ctx.sink.unlink_pull_request(viewer, reference).await?;
        }

        // Accepted and discarded — see the module comment.
        "thread.runtime-mode.set" | "thread.interaction-mode.set" => {}

        "thread.delete" => {
            return Err(CommandRefusal::Unsupported {
                command: tag,
                reason: "Archive it instead, or delete it from the Moatless UI.".to_string(),
            });
        }
        "project.meta.update" => project_meta_update(ctx, viewer, command).await?,
        "project.create" | "project.delete" => {
            return Err(CommandRefusal::Unsupported {
                command: tag,
                reason: "Projects are Moatless workspaces and are managed there.".to_string(),
            });
        }
        "thread.conversation.revert" => {
            let turn_count = command
                .get("turnCount")
                .and_then(Value::as_i64)
                .and_then(|n| i32::try_from(n).ok())
                .filter(|n| *n >= 0)
                .ok_or_else(|| missing("turnCount"))?;
            ctx.sink
                .revert(viewer, thread_id(command)?, turn_count)
                .await?;
        }
        "thread.checkpoint.revert" => {
            return Err(CommandRefusal::Unsupported {
                command: tag,
                reason: "This server takes no worktree snapshots, so files cannot be \
                         restored. Revert the conversation and keep the changes instead."
                    .to_string(),
            });
        }
        "thread.approval.respond" => {
            return Err(CommandRefusal::Unsupported {
                command: tag,
                reason: "Approvals are not part of this server.".to_string(),
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
    let attachments = attachments(message)?;

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
        let workspace_id = create
            .get("projectId")
            .and_then(Value::as_str)
            .ok_or_else(|| missing("bootstrap.createThread.projectId"))?
            .to_string();
        let skills = activated_skills(ctx, viewer, &workspace_id, &text).await;

        return ctx
            .sink
            .create_thread(
                viewer,
                NewThread {
                    task_id: task_id.clone(),
                    workspace_id,
                    instance,
                    model: model.clone(),
                    branch: branch(create),
                    first_message: Some(ThreadTurn {
                        task_id,
                        text,
                        message_uuid,
                        model,
                        agent_mode,
                        effort,
                        attachments,
                        skills,
                    }),
                    provision: false,
                },
            )
            .await;
    }

    let conversation = require_controllable(ctx, viewer, &task_id).await?;
    let agents = ctx.agents.list(viewer).await;
    let model = model_for_existing(&agents, &conversation, command)?;
    let skills = match conversation.workspace_id.as_deref() {
        Some(workspace_id) => activated_skills(ctx, viewer, workspace_id, &text).await,
        None => Vec::new(),
    };
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
                attachments,
                skills,
            },
        )
        .await
}

/// The skills a turn's text activates, as names Moatless frontloads.
///
/// The composer's `/` and `$` menus write a pick as a `$name` mention and leave
/// it in the prompt. Only a mention that names a skill this Workspace actually
/// offers counts, so the listing is read first — a `$HOME` in prose must not
/// activate anything. A prompt with no mention at all skips the read.
async fn activated_skills(
    ctx: &CommandContext<'_>,
    viewer: &ScopeContext,
    workspace_id: &str,
    text: &str,
) -> Vec<String> {
    if !skills::mentions_a_skill(text) {
        return Vec::new();
    }
    let offered = ctx
        .skills
        .list_for_workspace(viewer, ctx.caller, workspace_id)
        .await;
    let known = offered.iter().map(|skill| skill.name.as_str()).collect();
    skills::activated(text, &known)
}

/// The most attachments one turn may carry
/// (`contracts/src/orchestration.ts:144`).
const MAX_ATTACHMENTS: usize = 8;

/// The largest single image, decoded
/// (`contracts/src/orchestration.ts:145`). The composer re-encodes anything
/// bigger before it sends, so this is a backstop against a client that does
/// not, not a limit a person meets by pasting a screenshot.
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// `ChatAttachmentId`'s length (`contracts/src/orchestration.ts:175`).
const MAX_ATTACHMENT_ID_CHARS: usize = 128;

/// What the composer sent with a turn.
///
/// Two forms arrive here and both are the contract's. An attachment the client
/// uploaded first is a *reference* — an id and the name, type and size it
/// already told this server at mint time — and its bytes are already stored. An
/// attachment from a client that has not read the `attachmentUploads`
/// capability is *inline*: an image in the command as a `data:` URL, which is
/// where those bytes enter Moatless. The limits the client's own schema
/// enforces are re-checked on the inline form, and the mime type and size are
/// taken from the payload rather than from the sibling fields declaring them —
/// those are what the client believes, the bytes are what gets stored.
///
/// Anything wrong is a rejection rather than a silent drop: an image that
/// vanishes on send reads as Moatless having lost it.
fn attachments(message: &Value) -> Result<Vec<TurnAttachment>, CommandRefusal> {
    let Some(sent) = message.get("attachments").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    if sent.is_empty() {
        return Ok(Vec::new());
    }
    if sent.len() > MAX_ATTACHMENTS {
        return Err(CommandRefusal::Rejected {
            message: format!("A message may carry at most {MAX_ATTACHMENTS} attachments."),
        });
    }

    sent.iter().map(attachment).collect()
}

/// One attachment, in whichever form it arrived.
///
/// `ChatAttachment` is a union of `image`, `file` and a deliberately open
/// catch-all for a type this build does not know
/// (`contracts/src/orchestration.ts:240-244`). The catch-all exists so a newer
/// client does not make an older reader fail to decode the message; it is not
/// something this server can store, so it is refused by name rather than
/// guessed at.
///
/// Within the two known members, what tells the forms apart is `dataUrl`: only
/// the inline member carries one, and only an image may. Everything else names
/// an upload by id, and the bytes are already in storage.
fn attachment(sent: &Value) -> Result<TurnAttachment, CommandRefusal> {
    let kind = sent.get("type").and_then(Value::as_str).unwrap_or_default();
    if kind != "image" && kind != "file" {
        return Err(CommandRefusal::Rejected {
            message: format!("`{kind}` attachments are not supported."),
        });
    }

    let filename = sent
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| CommandRefusal::Rejected {
            message: "An attachment must have a name.".to_string(),
        })?;

    let Some(data_url) = sent.get("dataUrl").and_then(Value::as_str) else {
        let attachment_id = sent
            .get("id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| {
                !id.is_empty()
                    && id.len() <= MAX_ATTACHMENT_ID_CHARS
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
            })
            .ok_or_else(|| CommandRefusal::Rejected {
                message: format!("Attachment `{filename}` names no upload."),
            })?;
        return Ok(TurnAttachment::Uploaded {
            attachment_id: attachment_id.to_string(),
            kind: if kind == "image" {
                AttachmentKind::Image
            } else {
                AttachmentKind::File
            },
        });
    };

    // Only `UploadChatImageAttachment` carries bytes in the command
    // (`contracts/src/orchestration.ts:246`). A file has to be uploaded first,
    // which every client that can pick one already knows how to do.
    if kind != "image" {
        return Err(CommandRefusal::Rejected {
            message: format!("Attachment `{filename}` must be uploaded before it is sent."),
        });
    }

    let (mime_type, bytes) = decode_data_url(data_url).ok_or_else(|| CommandRefusal::Rejected {
        message: format!("Attachment `{filename}` is not a readable image payload."),
    })?;

    if !mime_type.starts_with("image/") {
        return Err(CommandRefusal::Rejected {
            message: format!("Attachment `{filename}` is not an image."),
        });
    }
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err(CommandRefusal::Rejected {
            message: format!("Attachment `{filename}` is empty or too large."),
        });
    }

    Ok(TurnAttachment::Inline {
        filename: filename.to_string(),
        mime_type,
        bytes,
    })
}

/// `data:<mime>;base64,<payload>` split into what it says it is and what it is.
///
/// Only base64 payloads are read: that is the one form the composer produces,
/// and a percent-encoded `data:` URL carrying image bytes is not something a
/// browser builds from a `File`.
fn decode_data_url(data_url: &str) -> Option<(String, Vec<u8>)> {
    let trimmed = data_url.trim();
    let rest = trimmed
        .strip_prefix("data:")
        .or_else(|| trimmed.strip_prefix("DATA:"))?;
    let (header, payload) = rest.split_once(',')?;

    let mut parameters = header.split(';');
    let mime_type = parameters.next().unwrap_or_default().trim().to_lowercase();
    if !parameters.any(|parameter| parameter.trim().eq_ignore_ascii_case("base64")) {
        return None;
    }

    // The composer chunks its own base64 and browsers wrap data URLs in
    // attributes, so whitespace inside the payload is ordinary rather than
    // corrupt.
    let payload: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&payload)
        .ok()?;

    Some((mime_type, bytes))
}

/// A thread created with nothing in it, from a proposed plan or from a draft
/// the person ran a Script in before they sent anything.
///
/// `provision` is what separates the two. A plan's thread is about to be sent
/// a turn, so it waits for the Sandbox; a draft's Script needs one now.
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
                model,
                branch: branch(command),
                first_message: None,
                provision: command
                    .get("provision")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            },
        )
        .await
}

/// Branch a new Thread off an existing one at one of its completed turns.
///
/// The fork's agent is fixed to the source Thread's — T3 has no picker for
/// switching it mid-branch — so a `modelSelection` override is checked
/// against the source's instance exactly as [`meta_update`] checks one
/// against an existing Thread's.
async fn fork_thread(
    ctx: &CommandContext<'_>,
    viewer: &ScopeContext,
    command: &Value,
) -> Result<(), CommandRefusal> {
    let new_task_id = thread_id(command)?.to_string();
    let source_task_id = command
        .get("sourceThreadId")
        .and_then(Value::as_str)
        .ok_or_else(|| missing("sourceThreadId"))?
        .to_string();
    let cut_at_turn = parse_at_turn(command)?;

    let source = ctx
        .conversations
        .readable_by_id(viewer, &source_task_id)
        .await
        .map_err(|error| {
            warn!(%error, task_id = %source_task_id, "T3 fork could not resolve its source thread");
            CommandRefusal::Failed {
                message: "Could not reach that thread.".to_string(),
            }
        })?
        .ok_or_else(|| CommandRefusal::NotWritable {
            task_id: source_task_id.clone(),
        })?;

    let agents = ctx.agents.list(viewer).await;
    let model = model_for_existing(&agents, &source, command)?;

    ctx.sink
        .fork(
            viewer,
            ForkThread {
                new_task_id,
                source_task_id,
                at_turn: cut_at_turn,
                message: command
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                same_sandbox: command
                    .get("sameSandbox")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                model,
                agent_mode: interaction_mode(command),
                effort: selected_effort(command),
            },
        )
        .await
}

/// The title a person's rename carries. A title asserted while the Task is
/// still unnamed is the client's own auto-title, and the agent names a Task on
/// its first turn — see this crate's `AGENTS.md`.
fn rename(conversation: &Conversation, command: &Value) -> Option<String> {
    let title = command.get("title").and_then(Value::as_str)?;
    conversation
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())?;
    Some(title.to_string())
}

/// The source Task's turn a fork cuts after, if the command named one.
/// `None` cuts after the newest turn that reached a terminal status.
fn parse_at_turn(command: &Value) -> Result<Option<i32>, CommandRefusal> {
    match command.get("atTurn") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_i64()
            .and_then(|n| i32::try_from(n).ok())
            .map(Some)
            .ok_or_else(|| CommandRefusal::Rejected {
                message: "`atTurn` must be a whole number.".to_string(),
            }),
    }
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
    let conversation = require_controllable(ctx, viewer, &task_id).await?;
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
                title: rename(&conversation, command),
                model,
            },
        )
        .await
}

/// The project fields this surface changes: its Workspace's Scripts, and
/// nothing else.
///
/// `project.meta.update` is one command carrying eight optional fields, and
/// only one of them names something this surface may set. The rest — the title,
/// the root path, the default model and effort, the icon, the favicon, the env
/// mode — describe a T3 project, which here is a *projection* of a Moatless
/// Workspace: its root is synthesized from the id, and the rest are Workspace
/// columns the settings page edits over REST. Accepting the command wholesale
/// and quietly dropping those would report a save that did not happen, so each
/// is refused by name.
async fn project_meta_update(
    ctx: &CommandContext<'_>,
    viewer: &ScopeContext,
    command: &Value,
) -> Result<(), CommandRefusal> {
    let workspace_id = command
        .get("projectId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| missing("projectId"))?;

    if let Some(field) = PROJECT_FIELDS_MOATLESS_OWNS
        .iter()
        .find(|field| command.get(**field).is_some_and(|value| !value.is_null()))
    {
        return Err(CommandRefusal::Unsupported {
            command: format!("project.meta.update ({field})"),
            reason: "A project is a Moatless workspace; change it there.".to_string(),
        });
    }

    let Some(scripts) = command.get("scripts") else {
        // Nothing asked for. Not an error — the client folds unrelated project
        // state into this command — and nothing to write.
        return Ok(());
    };

    scripts::replace(ctx.workspace_script_commands, viewer, workspace_id, scripts)
        .await
        .map_err(|error| match error {
            ScriptsWriteError::NotFound => CommandRefusal::Rejected {
                message: "That project is not one you can change from here.".to_string(),
            },
            ScriptsWriteError::GitSynced => CommandRefusal::Rejected {
                message: "This workspace is synced from Git. Edit \
                          `.moatless/workspaces.json`, or override the workspace in Moatless."
                    .to_string(),
            },
            ScriptsWriteError::Invalid(message) => CommandRefusal::Rejected { message },
            ScriptsWriteError::Database(error) => {
                warn!(%error, workspace_id, "Failed to write workspace scripts");
                CommandRefusal::Failed {
                    message: "The scripts could not be saved.".to_string(),
                }
            }
        })
}

/// The `project.meta.update` fields that describe a Moatless Workspace rather
/// than anything this surface may set.
const PROJECT_FIELDS_MOATLESS_OWNS: [&str; 6] = [
    "title",
    "workspaceRoot",
    "defaultModelSelection",
    "defaultThreadEnvMode",
    "faviconPath",
    "projectIcon",
];

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

/// The `ThreadPullRequestKey` a link or unlink command carries, on a Thread
/// this viewer may write to.
///
/// Gated on control rather than on read: a link changes which pull request every
/// surface reports for the Thread, and a reader may open one they cannot write.
async fn pull_request_ref(
    ctx: &CommandContext<'_>,
    viewer: &ScopeContext,
    command: &Value,
) -> Result<ThreadPullRequestRef, CommandRefusal> {
    let task_id = thread_id(command)?.to_string();
    require_controllable(ctx, viewer, &task_id).await?;

    let field = |name: &'static str| -> Result<String, CommandRefusal> {
        command
            .get(name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| missing(name))
    };
    let number = command
        .get("number")
        .and_then(Value::as_i64)
        .filter(|number| *number > 0)
        .ok_or_else(|| missing("number"))?;

    Ok(ThreadPullRequestRef {
        task_id,
        host: field("host")?,
        repository: field("repository")?,
        number,
    })
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

/// The Task, if this viewer may run a turn on it.
///
/// Resolved through the read a link resolves through, then gated on control —
/// which is the grant a turn takes, because a turn is the agent running in
/// someone's Sandbox and a controller can already run anything else there. A
/// Task that fails either is reported the same way as one that does not exist.
///
/// The status rule is this function's own, because the two paths want different
/// answers from it: a reader may open an archived conversation, and no one may
/// add a turn to one. Unarchiving is the way back, and it does not come through
/// here.
async fn require_controllable(
    ctx: &CommandContext<'_>,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<Conversation, CommandRefusal> {
    let unreachable = |error: sqlx::Error| {
        tracing::warn!(%error, task_id, "T3 command could not resolve its thread");
        CommandRefusal::Failed {
            message: "Could not reach that thread.".to_string(),
        }
    };
    let refused = || CommandRefusal::NotWritable {
        task_id: task_id.to_string(),
    };

    let conversation = ctx
        .conversations
        .readable_by_id(viewer, task_id)
        .await
        .map_err(unreachable)?
        .filter(|conversation| conversation.closed_at.is_none())
        .ok_or_else(refused)?;
    if !ctx
        .conversations
        .controls_conversation(viewer, task_id)
        .await
        .map_err(unreachable)?
    {
        return Err(refused());
    }
    Ok(conversation)
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
    use crate::ports::MockWorkspaceScriptCommands;
    use async_trait::async_trait;
    use moatless_types::agent::AgentType;
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
            /// Each attachment as it reached the sink, in the order it was
            /// sent.
            attachments: Vec<Attached>,
            skills: Vec<String>,
        },
        CreateThread {
            task_id: String,
            workspace_id: String,
            instance: String,
            branch: Option<String>,
            has_first_message: bool,
            provision: bool,
            skills: Vec<String>,
        },
        Fork {
            new_task_id: String,
            source_task_id: String,
            at_turn: Option<i32>,
            message: Option<String>,
            same_sandbox: bool,
            model: Option<String>,
            agent_mode: Option<String>,
            effort: Option<String>,
        },
        Revert {
            task_id: String,
            keep_through: i32,
        },
        UpdateMeta {
            task_id: String,
            title: Option<String>,
            model: Option<String>,
        },
        Interrupt {
            task_id: String,
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
        Follow {
            task_id: String,
        },
        Unfollow {
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
        SetVisibility {
            task_id: String,
            visibility: Visibility,
        },
        AnswerQuestion {
            task_id: String,
            tool_use_id: String,
            tool_name: String,
            payload: Option<Value>,
        },
        SetPullRequestLink {
            task_id: String,
            host: String,
            repository: String,
            number: i64,
            linked: bool,
        },
    }

    /// One attachment as the sink received it. An inline one is recorded by
    /// what its bytes turned out to be — not by what the client declared — and
    /// an uploaded one by the only thing it carries.
    #[derive(Debug, PartialEq, Eq)]
    enum Attached {
        Inline(String, String, usize),
        Uploaded(String, AttachmentKind),
    }

    impl From<TurnAttachment> for Attached {
        fn from(attachment: TurnAttachment) -> Self {
            match attachment {
                TurnAttachment::Inline {
                    filename,
                    mime_type,
                    bytes,
                } => Self::Inline(filename, mime_type, bytes.len()),
                TurnAttachment::Uploaded {
                    attachment_id,
                    kind,
                } => Self::Uploaded(attachment_id, kind),
            }
        }
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
                attachments: turn.attachments.into_iter().map(Attached::from).collect(),
                skills: turn.skills,
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
                provision: thread.provision,
                skills: thread
                    .first_message
                    .map(|message| message.skills)
                    .unwrap_or_default(),
            });
            Ok(())
        }
        async fn fork(
            &self,
            _viewer: &ScopeContext,
            fork: ForkThread,
        ) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::Fork {
                new_task_id: fork.new_task_id,
                source_task_id: fork.source_task_id,
                at_turn: fork.at_turn,
                message: fork.message,
                same_sandbox: fork.same_sandbox,
                model: fork.model,
                agent_mode: fork.agent_mode.map(|m| m.0),
                effort: fork.effort,
            });
            Ok(())
        }
        async fn revert(
            &self,
            _viewer: &ScopeContext,
            task_id: &str,
            keep_through: i32,
        ) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::Revert {
                task_id: task_id.to_string(),
                keep_through,
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
        async fn interrupt(
            &self,
            _viewer: &ScopeContext,
            task_id: &str,
        ) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::Interrupt {
                task_id: task_id.to_string(),
            });
            Ok(())
        }
        async fn stop(&self, _viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::Stop {
                task_id: task_id.to_string(),
            });
            Ok(())
        }
        async fn set_idle_timeout(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
            _minutes: i32,
        ) -> Result<(), CommandRefusal> {
            unreachable!("no `thread.*` command changes an idle timeout")
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
        async fn follow(
            &self,
            _viewer: &ScopeContext,
            task_id: &str,
        ) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::Follow {
                task_id: task_id.to_string(),
            });
            Ok(())
        }
        async fn unfollow(
            &self,
            _viewer: &ScopeContext,
            task_id: &str,
        ) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::Unfollow {
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
        async fn set_visibility(
            &self,
            _viewer: &ScopeContext,
            task_id: &str,
            visibility: Visibility,
        ) -> Result<(), CommandRefusal> {
            self.0.lock().unwrap().push(Call::SetVisibility {
                task_id: task_id.to_string(),
                visibility,
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
        async fn link_pull_request(
            &self,
            _viewer: &ScopeContext,
            reference: ThreadPullRequestRef,
        ) -> Result<(), CommandRefusal> {
            self.0
                .lock()
                .unwrap()
                .push(pull_request_call(reference, true));
            Ok(())
        }
        async fn unlink_pull_request(
            &self,
            _viewer: &ScopeContext,
            reference: ThreadPullRequestRef,
        ) -> Result<(), CommandRefusal> {
            self.0
                .lock()
                .unwrap()
                .push(pull_request_call(reference, false));
            Ok(())
        }
    }

    fn pull_request_call(reference: ThreadPullRequestRef, linked: bool) -> Call {
        Call::SetPullRequestLink {
            task_id: reference.task_id,
            host: reference.host,
            repository: reference.repository,
            number: reference.number,
            linked,
        }
    }

    /// The two skills `ws-1` offers every test here.
    struct StubSkills;

    #[async_trait]
    impl SkillCatalog for StubSkills {
        async fn list_for_workspace(
            &self,
            _viewer: &ScopeContext,
            _caller: Uuid,
            _workspace_id: &str,
        ) -> Vec<crate::ports::WorkspaceSkill> {
            ["git-workflow", "linear"]
                .into_iter()
                .map(|name| crate::ports::WorkspaceSkill {
                    name: name.to_string(),
                    description: format!("The {name} skill"),
                    plugin_name: None,
                })
                .collect()
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
            visibility: Visibility::Private,
            owner_name: None,
            turn_number: None,
            turn_status: None,
            turn_started_at: None,
            turn_ended_at: None,
            sandbox_desired_state_raw: None,
            sandbox_stop_reason_raw: None,
            sandbox_started_at: None,
            sandbox_updated_at: None,
            running_commands: 0,
            latest_user_message_at: None,
            assistant_model: None,
            requested_model: None,
            pending_question: false,
            pending_plan: false,
            settled_override: None,
            settled_at: None,
            snoozed_until: None,
            snoozed_at: None,
            pull_requests: None,
        }
    }

    fn named(name: &str) -> Conversation {
        Conversation {
            name: Some(name.to_string()),
            ..conversation(AgentType::CLAUDE_CODE)
        }
    }

    /// A DAO that answers with one conversation, or with none when `found` is
    /// `None` — which is what "the viewer cannot be shown it" looks like from
    /// here. Control is granted; the test that withholds it overrides
    /// `controls_conversation` itself.
    fn conversations(found: Option<Conversation>) -> MockConversationDao {
        let mut dao = MockConversationDao::new();
        dao.expect_readable_by_id()
            .returning(move |_, _| Ok(found.clone()));
        dao.expect_controls_conversation()
            .returning(|_, _| Ok(true));
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
        dispatch_with_workspaces(dao, MockWorkspaceScriptCommands::new(), command).await
    }

    async fn dispatch_with_workspaces(
        dao: MockConversationDao,
        workspaces: MockWorkspaceScriptCommands,
        command: Value,
    ) -> (Result<Value, CommandRefusal>, Vec<Call>) {
        let sink = RecordingSink::default();
        let agents = StubAgents;
        let skills = StubSkills;
        let ctx = CommandContext {
            conversations: &dao,
            agents: &agents,
            sink: &sink,
            workspace_script_commands: &workspaces,
            skills: &skills,
            caller: Uuid::nil(),
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
                attachments: vec![],
                skills: vec![],
            }]
        );
    }

    /// A turn whose prompt reads the way the composer leaves it after a pick.
    fn turn_saying(text: &str) -> Value {
        let mut command = turn(None);
        command["message"]["text"] = json!(text);
        command
    }

    /// The `$name` stays in the text, because that is what the T3 timeline
    /// renders as a chip. Moatless frontloads by name instead, so the same pick
    /// has to reach `skills` as well or the prompt is the only thing that ships.
    #[tokio::test]
    async fn a_turn_activates_the_skill_its_text_mentions() {
        let (result, calls) = run(turn_saying("$git-workflow open the PR")).await;
        assert!(result.is_ok());
        assert!(
            matches!(calls.as_slice(), [Call::SendTurn { skills, .. }] if skills == &["git-workflow"]),
            "expected git-workflow activated, got {calls:?}"
        );
    }

    /// A mention only counts when the Workspace offers it, so an env var in
    /// prose stays prose.
    #[tokio::test]
    async fn a_mention_of_something_that_is_not_a_skill_activates_nothing() {
        let (_, calls) = run(turn_saying("echo $HOME and $linearity")).await;
        assert!(
            matches!(calls.as_slice(), [Call::SendTurn { skills, .. }] if skills.is_empty()),
            "expected no skills, got {calls:?}"
        );
    }

    /// The create-thread composer is the one the person picks from most, and it
    /// sends its pick on the bootstrapped turn that creates the Task.
    #[tokio::test]
    async fn a_bootstrapped_turn_activates_the_skills_it_mentions() {
        let mut command = turn_saying("$linear then $git-workflow");
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
        let (result, calls) = run_against(command, None).await;
        assert!(result.is_ok());
        assert!(
            matches!(
                calls.as_slice(),
                [Call::CreateThread { skills, .. }] if skills == &["linear", "git-workflow"]
            ),
            "expected both picks in the order they were made, got {calls:?}"
        );
    }

    /// A one-pixel PNG, as the composer would send it.
    fn an_image(name: &str) -> Value {
        json!({
            "type": "image",
            "name": name,
            "mimeType": "image/png",
            "sizeBytes": 70,
            "dataUrl": format!("data:image/png;base64,{PNG_BASE64}")
        })
    }

    /// The 70 bytes of a 1×1 transparent PNG.
    const PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk\
                              YPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

    /// A turn carrying `attachments`.
    fn turn_with(attachments: Value) -> Value {
        let mut command = turn(None);
        command["message"]["attachments"] = attachments;
        command
    }

    /// The bytes are what reaches the sink — not the data URL, and not the
    /// sizes the client declared, which are only what the client believes.
    #[tokio::test]
    async fn an_attached_image_reaches_the_sink_decoded() {
        let (result, calls) = run(turn_with(json!([an_image("screenshot.png")]))).await;
        assert!(result.is_ok());
        let Some(Call::SendTurn { attachments, .. }) = calls.first() else {
            panic!("expected a turn, got {calls:?}");
        };
        assert_eq!(
            attachments,
            &vec![Attached::Inline(
                "screenshot.png".to_string(),
                "image/png".to_string(),
                70
            )]
        );
    }

    /// The order the composer held them in is the order they are stored in, so
    /// the bubble renders them the way the person arranged them.
    #[tokio::test]
    async fn attachments_keep_the_order_they_were_sent_in() {
        let (_, calls) = run(turn_with(json!([an_image("a.png"), an_image("b.png")]))).await;
        let Some(Call::SendTurn { attachments, .. }) = calls.first() else {
            panic!("expected a turn, got {calls:?}");
        };
        assert_eq!(
            attachments,
            &vec![
                Attached::Inline("a.png".to_string(), "image/png".to_string(), 70),
                Attached::Inline("b.png".to_string(), "image/png".to_string(), 70),
            ]
        );
    }

    /// A composer that has read `attachmentUploads` uploads first and sends a
    /// reference, for images as well as files. Both members carry no bytes, and
    /// the id is the only thing this side needs off the wire — the name, type
    /// and size beside it are the client's copy of what the upload row already
    /// records.
    ///
    /// The member travels with the id. An id is not a provenance, so the
    /// resolver has to measure the stored row against the member it was claimed
    /// under, and it can only do that if this side says which one that was.
    #[tokio::test]
    async fn an_uploaded_attachment_reaches_the_sink_as_the_id_it_named() {
        let uploaded = |kind: &str, name: &str, id: &str| {
            json!({
                "type": kind,
                "id": id,
                "name": name,
                "mimeType": if kind == "image" { "image/png" } else { "application/pdf" },
                "sizeBytes": 4096
            })
        };
        let (result, calls) = run(turn_with(json!([
            uploaded("image", "shot.png", "6b1f7b2e-0000-4000-8000-000000000001"),
            uploaded("file", "report.pdf", "6b1f7b2e-0000-4000-8000-000000000002"),
        ])))
        .await;

        assert!(result.is_ok(), "{result:?}");
        let Some(Call::SendTurn { attachments, .. }) = calls.first() else {
            panic!("expected a turn, got {calls:?}");
        };
        assert_eq!(
            attachments,
            &vec![
                Attached::Uploaded(
                    "6b1f7b2e-0000-4000-8000-000000000001".to_string(),
                    AttachmentKind::Image
                ),
                Attached::Uploaded(
                    "6b1f7b2e-0000-4000-8000-000000000002".to_string(),
                    AttachmentKind::File
                ),
            ]
        );
    }

    /// A file has no inline form — only `UploadChatImageAttachment` carries
    /// bytes — and an id that is a path is not an id. Both refuse the turn
    /// rather than reaching the sink with something it would have to guess at.
    #[tokio::test]
    async fn a_reference_that_is_not_one_refuses_the_turn() {
        for attachment in [
            // A file with bytes in the command. No client sends this; one that
            // did would be sending a member of a union that does not exist.
            json!({
                "type": "file",
                "name": "notes.txt",
                "mimeType": "text/plain",
                "sizeBytes": 5,
                "dataUrl": "data:text/plain;base64,aGVsbG8="
            }),
            // Neither bytes nor an id.
            json!({ "type": "file", "name": "report.pdf", "mimeType": "application/pdf" }),
            // Outside `ChatAttachmentId`'s alphabet: an id that could leave the
            // route it becomes a segment of.
            json!({ "type": "file", "id": "../secrets", "name": "a.pdf", "mimeType": "application/pdf" }),
            json!({ "type": "image", "id": "", "name": "a.png", "mimeType": "image/png" }),
        ] {
            let (result, calls) = run(turn_with(json!([attachment.clone()]))).await;
            assert!(
                matches!(result, Err(CommandRefusal::Rejected { .. })),
                "expected {attachment} to be rejected, got {result:?}"
            );
            assert!(calls.is_empty(), "expected no write for {attachment}");
        }
    }

    /// A turn with nothing attached is byte-identical to one sent before this
    /// surface read attachments at all.
    #[tokio::test]
    async fn a_turn_without_attachments_carries_none() {
        let (_, calls) = run(turn(None)).await;
        assert!(matches!(
            calls.first(),
            Some(Call::SendTurn { attachments, .. }) if attachments.is_empty()
        ));
    }

    /// Each of these is refused rather than dropped: an image that disappears
    /// on send reads as Moatless losing it, and nothing reaches the sink, so
    /// the refusal is the whole outcome.
    #[tokio::test]
    async fn an_attachment_that_cannot_be_honoured_refuses_the_turn() {
        let mut oversized = an_image("huge.png");
        oversized["dataUrl"] = json!(format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(vec![0u8; MAX_IMAGE_BYTES + 1])
        ));

        let mut not_an_image = an_image("notes.txt");
        not_an_image["dataUrl"] = json!("data:text/plain;base64,aGVsbG8=");

        let mut unreadable = an_image("broken.png");
        unreadable["dataUrl"] = json!("data:image/png;base64,%%%not-base64%%%");

        let mut nameless = an_image("x.png");
        nameless["name"] = json!("   ");

        let mut future_kind = an_image("clip.mp4");
        future_kind["type"] = json!("video");

        for attachment in [
            oversized,
            not_an_image,
            unreadable,
            nameless,
            future_kind,
            json!({ "type": "image", "name": "no-payload.png" }),
        ] {
            let name = attachment["name"].clone();
            let (result, calls) = run(turn_with(json!([attachment]))).await;
            assert!(
                matches!(result, Err(CommandRefusal::Rejected { .. })),
                "expected {name} to be rejected, got {result:?}"
            );
            assert!(calls.is_empty(), "expected no write for {name}");
        }
    }

    /// The contract caps a turn at eight, and a client that sends more is
    /// answering a different contract.
    #[tokio::test]
    async fn more_attachments_than_the_contract_allows_refuses_the_turn() {
        let too_many: Vec<Value> = (0..=MAX_ATTACHMENTS)
            .map(|index| an_image(&format!("{index}.png")))
            .collect();
        let (result, calls) = run(turn_with(Value::Array(too_many))).await;
        assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
        assert!(calls.is_empty());
    }

    /// A data URL that is not base64 has no bytes to read, and a payload with
    /// the newlines an encoder wrapped it at does.
    #[test]
    fn a_data_url_is_read_only_where_it_is_base64() {
        assert!(decode_data_url("data:image/png,not-encoded").is_none());
        assert!(decode_data_url("https://example.com/a.png").is_none());
        assert!(decode_data_url("data:image/png;base64").is_none());

        let (mime_type, bytes) =
            decode_data_url("data:image/PNG;base64,aGVs\n bG8=").expect("decoded");
        assert_eq!(mime_type, "image/png");
        assert_eq!(bytes, b"hello");
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
                attachments: vec![],
                skills: vec![],
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
    async fn a_thread_the_viewer_cannot_be_shown_is_refused() {
        let (result, calls) = run_against(turn(None), None).await;
        assert!(matches!(result, Err(CommandRefusal::NotWritable { .. })));
        assert!(calls.is_empty());
    }

    /// Reading a conversation is not running a turn on it. The two refusals are
    /// the same on the wire, deliberately: which one it was says who owns the
    /// Task.
    #[tokio::test]
    async fn a_thread_the_viewer_reads_but_does_not_control_is_refused() {
        let found = conversation(AgentType::CLAUDE_CODE);
        let mut dao = MockConversationDao::new();
        dao.expect_readable_by_id()
            .returning(move |_, _| Ok(Some(found.clone())));
        dao.expect_controls_conversation()
            .returning(|_, _| Ok(false));
        dao.expect_interactive_call()
            .returning(|_, _| Ok(Some(asked(mapping::QUESTION_TOOL))));
        let (result, calls) = dispatch_against(dao, turn(None)).await;
        assert!(matches!(result, Err(CommandRefusal::NotWritable { .. })));
        assert!(calls.is_empty());
    }

    /// An archived conversation resolves — the viewer follows it and may read
    /// it, which is what lets them open it — and a turn into it is still
    /// refused. Unarchiving is the way back, and it does not come through here.
    #[tokio::test]
    async fn a_turn_into_an_archived_thread_is_refused() {
        let archived = Conversation {
            closed_at: Some(Utc::now()),
            ..conversation(AgentType::CLAUDE_CODE)
        };
        let (result, calls) = run_against(turn(None), Some(archived)).await;
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
                provision: false,
                skills: vec![],
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
                provision: false,
                skills: vec![],
            }]
        );
    }

    /// The draft's Script run: the Task is created for the Sandbox rather than
    /// for a turn, so the create is the only thing that will ever ask for one.
    #[tokio::test]
    async fn a_standalone_create_can_ask_for_a_sandbox() {
        let mut command = json!({
            "type": "thread.create",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "projectId": "ws-1",
            "title": "Dev server",
            "modelSelection": { "instanceId": "codex", "model": "gpt-5.6-sol" },
            "runtimeMode": "full-access",
            "interactionMode": "default",
            "branch": null,
            "worktreePath": null,
            "createdAt": "2026-07-01T09:00:00.000Z"
        });
        command["provision"] = json!(true);
        let (result, calls) = run_against(command, None).await;
        assert!(result.is_ok());
        assert!(matches!(
            calls.as_slice(),
            [Call::CreateThread { provision: true, .. }]
        ));
    }

    /// The command a hover-icon fork sends with nothing else picked: the
    /// source thread and where to cut, nothing more.
    fn fork_command() -> Value {
        json!({
            "type": "thread.fork",
            "commandId": "cmd-1",
            "threadId": "fork-1",
            "sourceThreadId": "task-1",
            "createdAt": "2026-07-01T09:00:00.000Z"
        })
    }

    #[tokio::test]
    async fn a_fork_persists_under_the_id_the_client_minted() {
        let (result, calls) = run(fork_command()).await;
        assert!(result.is_ok());
        assert_eq!(
            calls,
            vec![Call::Fork {
                new_task_id: "fork-1".to_string(),
                source_task_id: "task-1".to_string(),
                at_turn: None,
                message: None,
                same_sandbox: false,
                model: None,
                agent_mode: None,
                effort: None,
            }]
        );
    }

    /// `atTurn`, `sameSandbox` and `message` reach the sink verbatim — the
    /// same shape the HTTP fork request carries. A `branch` in the command is
    /// dropped: a fork runs on the branch its source is already on.
    #[tokio::test]
    async fn a_fork_carries_at_turn_same_sandbox_and_message_but_not_branch() {
        let mut command = fork_command();
        command["atTurn"] = json!(3);
        command["sameSandbox"] = json!(false);
        command["branch"] = json!("feat/try-again");
        command["message"] = json!("try a different approach");
        let (result, calls) = run(command).await;
        assert!(result.is_ok());
        assert_eq!(
            calls,
            vec![Call::Fork {
                new_task_id: "fork-1".to_string(),
                source_task_id: "task-1".to_string(),
                at_turn: Some(3),
                message: Some("try a different approach".to_string()),
                same_sandbox: false,
                model: None,
                agent_mode: None,
                effort: None,
            }]
        );
    }

    /// A model override is checked and translated against the *source*
    /// thread's agent, the same way `meta_update` checks one against the
    /// thread it is updating — the fork has no agent of its own yet.
    #[tokio::test]
    async fn a_fork_carries_a_translated_model_override() {
        let mut command = fork_command();
        command["modelSelection"] =
            json!({ "instanceId": "claudeAgent", "model": "claude-haiku-4-5" });
        let (result, calls) = run(command).await;
        assert!(result.is_ok());
        assert!(matches!(
            calls.as_slice(),
            [Call::Fork { model, .. }] if model.as_deref() == Some("haiku")
        ));
    }

    #[tokio::test]
    async fn a_fork_without_a_source_thread_id_is_refused() {
        let mut command = fork_command();
        command.as_object_mut().unwrap().remove("sourceThreadId");
        let (result, calls) = run(command).await;
        assert!(calls.is_empty());
        assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
    }

    #[tokio::test]
    async fn a_fork_at_turn_that_is_not_a_whole_number_is_refused() {
        let mut command = fork_command();
        command["atTurn"] = json!("three");
        let (result, calls) = run(command).await;
        assert!(calls.is_empty());
        assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
    }

    /// A source thread the viewer cannot read is reported the same as one
    /// that does not exist — existence hiding, as everywhere else on this
    /// surface.
    #[tokio::test]
    async fn a_fork_of_a_thread_the_viewer_cannot_read_is_refused() {
        let (result, calls) = run_against(fork_command(), None).await;
        assert!(calls.is_empty());
        assert!(matches!(
            result,
            Err(CommandRefusal::NotWritable { task_id }) if task_id == "task-1"
        ));
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
        let (result, calls) = run_against(command, Some(named("Checkout bug"))).await;
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

    /// The client titles a thread from its first message seconds after creating
    /// it, before the agent has run. That title is dropped, so the Task reaches
    /// the pod unnamed and the agent names it. The model still lands.
    #[tokio::test]
    async fn a_title_asserted_while_the_thread_is_unnamed_is_dropped() {
        let command = json!({
            "type": "thread.meta.update",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "title": "Doesnt the agent in the sandbox set a task name on...",
            "modelSelection": { "instanceId": "claudeAgent", "model": "claude-haiku-4-5" }
        });
        let (result, calls) = run(command).await;
        assert!(result.is_ok());
        assert_eq!(
            calls,
            vec![Call::UpdateMeta {
                task_id: "task-1".to_string(),
                title: None,
                model: Some("haiku".to_string()),
            }]
        );
    }

    /// A name of blanks is no name: the auto-title would otherwise pass through
    /// whenever something wrote one.
    #[tokio::test]
    async fn a_title_asserted_against_a_blank_name_is_dropped() {
        let command = json!({
            "type": "thread.meta.update",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "title": "Fix the checkout"
        });
        let (result, calls) = run_against(command, Some(named("   "))).await;
        assert!(result.is_ok());
        assert_eq!(
            calls,
            vec![Call::UpdateMeta {
                task_id: "task-1".to_string(),
                title: None,
                model: None,
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
        let (result, calls) = run_against(command, Some(named("Checkout bug"))).await;
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

    /// The Stop button under a running turn sends the interrupt, and it must
    /// reach the agent-only stop: a person ending a turn keeps their Sandbox.
    /// A session stop is the one that releases it.
    #[tokio::test]
    async fn interrupt_stops_the_agent_and_session_stop_stops_the_sandbox() {
        for (tag, expected) in [
            (
                "thread.turn.interrupt",
                Call::Interrupt {
                    task_id: "task-1".to_string(),
                },
            ),
            (
                "thread.session.stop",
                Call::Stop {
                    task_id: "task-1".to_string(),
                },
            ),
        ] {
            let command = json!({
                "type": tag,
                "commandId": "cmd-1",
                "threadId": "task-1",
                "createdAt": "2026-07-01T09:00:00.000Z"
            });
            let (result, calls) = run(command).await;
            assert!(result.is_ok(), "{tag}");
            assert_eq!(calls, vec![expected], "{tag}");
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
    async fn following_and_unfollowing_reach_the_sink() {
        for (tag, expected) in [
            (
                "thread.follow",
                Call::Follow {
                    task_id: "task-1".to_string(),
                },
            ),
            (
                "thread.unfollow",
                Call::Unfollow {
                    task_id: "task-1".to_string(),
                },
            ),
        ] {
            let command = json!({
                "type": tag,
                "commandId": "cmd-1",
                "threadId": "task-1",
            });
            let (result, calls) = run(command).await;
            assert!(result.is_ok(), "{tag}");
            assert_eq!(calls, vec![expected], "{tag}");
        }
    }

    #[tokio::test]
    async fn a_visibility_change_reaches_the_sink_typed() {
        for value in ["public", "private"] {
            let command = json!({
                "type": "thread.visibility.set",
                "commandId": "cmd-1",
                "threadId": "task-1",
                "visibility": value,
            });
            let (result, calls) = run(command).await;
            assert!(result.is_ok(), "{value}");
            assert_eq!(
                calls,
                vec![Call::SetVisibility {
                    task_id: "task-1".to_string(),
                    visibility: Visibility::try_from(value).unwrap(),
                }],
                "{value}"
            );
        }
    }

    #[tokio::test]
    async fn a_visibility_change_rejects_an_unknown_level_before_any_write() {
        for command in [
            json!({
                "type": "thread.visibility.set",
                "commandId": "cmd-1",
                "threadId": "task-1",
            }),
            json!({
                "type": "thread.visibility.set",
                "commandId": "cmd-1",
                "threadId": "task-1",
                "visibility": "secret",
            }),
        ] {
            let (result, calls) = run(command).await;
            assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
            assert!(calls.is_empty());
        }
    }

    /// The key a link or an unlink names.
    fn pull_request_command(kind: &str) -> Value {
        json!({
            "type": format!("thread.pull-request.{kind}"),
            "commandId": "cmd-1",
            "threadId": "task-1",
            "host": "github.com",
            "repository": "acme/web",
            "number": 42,
            "url": "https://github.com/acme/web/pull/42",
            "source": "manual",
        })
    }

    #[tokio::test]
    async fn a_link_and_an_unlink_reach_the_sink_as_the_same_key() {
        for (kind, linked) in [("link", true), ("unlink", false)] {
            let (result, calls) = run(pull_request_command(kind)).await;
            assert!(result.is_ok(), "{kind}");
            assert_eq!(
                calls,
                vec![Call::SetPullRequestLink {
                    task_id: "task-1".to_string(),
                    host: "github.com".to_string(),
                    repository: "acme/web".to_string(),
                    number: 42,
                    linked,
                }],
                "{kind}"
            );
        }
    }

    /// A key the server cannot resolve to one pull request is refused before
    /// anything writes a Binding, because a link changes what every surface
    /// reports for the Thread.
    #[tokio::test]
    async fn a_link_naming_no_pull_request_is_rejected_before_any_write() {
        for absent in ["host", "repository", "number"] {
            let mut command = pull_request_command("link");
            command[absent] = Value::Null;
            let (result, calls) = run(command).await;
            assert!(
                matches!(result, Err(CommandRefusal::Rejected { .. })),
                "{absent}"
            );
            assert!(calls.is_empty(), "{absent}");
        }
    }

    /// Gated on control rather than on read: a reader may open a Thread they
    /// cannot write, and linking a pull request is a write.
    #[tokio::test]
    async fn a_link_into_a_thread_the_viewer_does_not_control_is_refused() {
        let found = conversation(AgentType::CLAUDE_CODE);
        let mut dao = MockConversationDao::new();
        dao.expect_readable_by_id()
            .returning(move |_, _| Ok(Some(found.clone())));
        dao.expect_controls_conversation()
            .returning(|_, _| Ok(false));
        let (result, calls) = dispatch_against(dao, pull_request_command("link")).await;
        assert!(matches!(result, Err(CommandRefusal::NotWritable { .. })));
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

    /// `turnCount` is the cut itself and reaches the sink unchanged: it is the
    /// `checkpointTurnCount` this server reported, which is a `task_turns`
    /// number, so a translation on either side would move the cut.
    #[tokio::test]
    async fn a_conversation_revert_carries_its_turn_count_through() {
        let (result, calls) = run(json!({
            "type": "thread.conversation.revert",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "turnCount": 4
        }))
        .await;
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            calls,
            vec![Call::Revert {
                task_id: "task-1".to_string(),
                keep_through: 4
            }]
        );
    }

    /// `0` empties the thread, and is a cut like any other.
    #[tokio::test]
    async fn a_conversation_revert_to_zero_reaches_the_sink() {
        let (result, calls) = run(json!({
            "type": "thread.conversation.revert",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "turnCount": 0
        }))
        .await;
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(
            calls,
            vec![Call::Revert {
                task_id: "task-1".to_string(),
                keep_through: 0
            }]
        );
    }

    #[tokio::test]
    async fn a_conversation_revert_without_a_usable_turn_count_writes_nothing() {
        for turn_count in [json!(-1), json!("4"), Value::Null] {
            let (result, calls) = run(json!({
                "type": "thread.conversation.revert",
                "commandId": "cmd-1",
                "threadId": "task-1",
                "turnCount": turn_count
            }))
            .await;
            assert!(
                matches!(result, Err(CommandRefusal::Rejected { .. })),
                "{turn_count} was not rejected: {result:?}"
            );
            assert!(calls.is_empty(), "{turn_count} reached the sink");
        }
    }

    /// A `project.meta.update` carrying `scripts`, as the client's scripts menu
    /// sends it — the whole list as it should now be.
    fn scripts_update(scripts: Value) -> Value {
        json!({
            "type": "project.meta.update",
            "commandId": "cmd-1",
            "projectId": "ws-1",
            "scripts": scripts
        })
    }

    fn wire_script(id: &str, command: &str) -> Value {
        json!({"id": id, "name": id, "command": command, "icon": "play", "runOnWorktreeCreate": false})
    }

    #[tokio::test]
    async fn script_edits_decode_wire_fields_and_delegate_to_workspace_owner() {
        let mut commands = MockWorkspaceScriptCommands::new();
        commands
            .expect_replace_scripts()
            .withf(|_, workspace_id, scripts| {
                workspace_id == "ws-1"
                    && scripts.len() == 1
                    && scripts[0].name == "dev"
                    && scripts[0].command == "pnpm dev"
                    && scripts[0].port == Some(3000)
                    && scripts[0].env_vars.is_none()
                    && scripts[0].run_on_start
            })
            .once()
            .returning(|_, _, _| Ok(()));
        let mut script = wire_script("dev", "pnpm dev");
        script["port"] = json!(3000);
        script["runOnWorktreeCreate"] = json!(true);
        let (result, calls) = dispatch_with_workspaces(
            conversations(None),
            commands,
            scripts_update(json!([script])),
        )
        .await;
        assert!(result.is_ok(), "{result:?}");
        assert!(calls.is_empty());
    }

    #[tokio::test]
    async fn workspace_write_refusals_keep_the_protocol_mapping() {
        for (error, message_part) in [
            (ScriptsWriteError::GitSynced, "workspaces.json"),
            (ScriptsWriteError::NotFound, "not one you can change"),
            (
                ScriptsWriteError::Invalid("duplicate name".into()),
                "duplicate name",
            ),
        ] {
            let mut commands = MockWorkspaceScriptCommands::new();
            commands
                .expect_replace_scripts()
                .once()
                .return_once(|_, _, _| Err(error));
            let (result, _) = dispatch_with_workspaces(
                conversations(None),
                commands,
                scripts_update(json!([wire_script("dev", "pnpm dev")])),
            )
            .await;
            let Err(CommandRefusal::Rejected { message }) = result else {
                panic!("expected rejection: {result:?}")
            };
            assert!(message.contains(message_part), "{message}");
        }
    }

    #[tokio::test]
    async fn invalid_wire_script_never_reaches_workspace_owner() {
        let mut commands = MockWorkspaceScriptCommands::new();
        commands.expect_replace_scripts().never();
        let mut script = wire_script("dev", "pnpm dev");
        script["port"] = json!(70000);
        let (result, _) = dispatch_with_workspaces(
            conversations(None),
            commands,
            scripts_update(json!([script])),
        )
        .await;
        assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
    }

    #[tokio::test]
    async fn project_fields_owned_by_moatless_are_refused() {
        for field in ["title", "workspaceRoot", "faviconPath"] {
            let mut command = scripts_update(json!([]));
            command[field] = json!("something");
            let (result, _) = dispatch_with_workspaces(
                conversations(None),
                MockWorkspaceScriptCommands::new(),
                command,
            )
            .await;
            assert!(
                matches!(result, Err(CommandRefusal::Unsupported { .. })),
                "{result:?}"
            );
        }
    }

    #[tokio::test]
    async fn project_update_without_scripts_does_not_write() {
        let mut commands = MockWorkspaceScriptCommands::new();
        commands.expect_replace_scripts().never();
        let (result, _) = dispatch_with_workspaces(
            conversations(None),
            commands,
            json!({"type": "project.meta.update", "commandId": "cmd-1", "projectId": "ws-1"}),
        )
        .await;
        assert!(result.is_ok(), "{result:?}");
    }

    #[tokio::test]
    async fn a_successful_command_reports_the_listing_position() {
        let (result, _) = run(turn(None)).await;
        assert_eq!(result.unwrap(), json!({ "sequence": 42 }));
    }
}
