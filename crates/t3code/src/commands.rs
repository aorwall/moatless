//! `orchestration.dispatchCommand` — the one write the T3 client makes.
//!
//! T3's command union is wider than Moatless's: this module states, per
//! command, whether it is carried out, discarded, or refused — the two look
//! identical here but not to the person holding the mouse.
//!
//! Discarded on purpose: `thread.runtime-mode.set`,
//! `thread.interaction-mode.set`, and
//! `bootstrap.prepareWorktree`/`runSetupScript` (the Sandbox's business).

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use tracing::warn;

use moatless_types::access_control::ScopeContext;

use base64::Engine;

use super::dao::{Conversation, ConversationDao};
use super::ports::{
    AgentCatalogSource, AttachmentKind, CommandRefusal, ForkThread, NewThread, QuestionAnswer,
    QueueChange, SkillCatalog, ThreadCommandSink, ThreadMeta, ThreadPullRequestRef, ThreadTurn,
    TurnAttachment,
};
use super::providers;
use crate::message_context::inline_message_text;
use crate::skills;
use moatless_types::activity as mapping;
use moatless_types::agent::{AgentMode, Harness};
use moatless_types::task::Visibility;
use uuid::Uuid;

/// What the surface needs in order to answer a command.
pub struct CommandContext<'a> {
    pub conversations: &'a dyn ConversationDao,
    pub agents: &'a dyn AgentCatalogSource,
    pub sink: &'a dyn ThreadCommandSink,
    /// The skills of the Workspace a turn runs in, for resolving the `$name`
    /// mentions the composer leaves in the prompt.
    pub skills: &'a dyn SkillCatalog,
    /// The viewer, as the id plugin activation records are keyed by — see
    /// [`SkillCatalog::list_for_workspace`].
    pub caller: Uuid,
}

/// Answers one dispatched command, returning the contract's `DispatchResult`.
///
/// `sequence` is the listing position reached, so the client can tell whether
/// its own change already shows in what it holds.
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
        "thread.queuedTurn.steer" => {
            let task_id = thread_id(command)?;
            let message_uuid = command
                .get("messageId")
                .and_then(Value::as_str)
                .ok_or_else(|| missing("messageId"))?;
            require_controllable(ctx, viewer, task_id).await?;
            ctx.sink.steer_queued(viewer, task_id, message_uuid).await?;
        }
        "thread.queuedTurn.cancel"
        | "thread.queuedTurn.edit"
        | "thread.queuedTurn.reorder"
        | "thread.queue.resume" => change_queue(ctx, viewer, &tag, command).await?,
        "thread.visit" => {
            let visited_at = command
                .get("visitedAt")
                .and_then(Value::as_str)
                .ok_or_else(|| missing("visitedAt"))?;
            let visited_at = DateTime::parse_from_rfc3339(visited_at)
                .map_err(|_| CommandRefusal::Rejected {
                    message: format!("`{visited_at}` is not a timestamp."),
                })?
                .with_timezone(&Utc);
            // A clock ahead of ours must not hide activity still to come.
            let visited_at = visited_at.min(Utc::now());
            ctx.sink
                .visit(viewer, thread_id(command)?, visited_at)
                .await?;
        }
        "thread.mark-unread" => {
            ctx.sink.mark_unread(viewer, thread_id(command)?).await?;
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
        "thread.auto-settle.set" => {
            let enabled = command
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or_else(|| missing("enabled"))?;
            ctx.sink
                .set_auto_settle(viewer, thread_id(command)?, enabled)
                .await?;
        }
        "thread.pin" => {
            let order_key = optional_order_key(command)?;
            ctx.sink.pin(viewer, thread_id(command)?, order_key).await?;
        }
        "thread.unpin" => {
            ctx.sink.unpin(viewer, thread_id(command)?).await?;
        }
        "thread.pin.reorder" => {
            let order_key = optional_order_key(command)?.ok_or_else(|| missing("orderKey"))?;
            ctx.sink
                .set_pin_order_key(viewer, thread_id(command)?, order_key)
                .await?;
        }
        "thread.active.reorder" => {
            let order_key = optional_order_key(command)?.ok_or_else(|| missing("orderKey"))?;
            ctx.sink
                .set_active_order_key(viewer, thread_id(command)?, order_key)
                .await?;
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
        "thread.pull-request.watch" => {
            let watching = command
                .get("watching")
                .and_then(Value::as_bool)
                .ok_or_else(|| missing("watching"))?;
            let reference = pull_request_ref(ctx, viewer, command).await?;
            ctx.sink
                .watch_pull_request(viewer, reference, watching)
                .await?;
        }

        // Accepted and discarded — see the module comment.
        "thread.runtime-mode.set" | "thread.interaction-mode.set" => {}

        "thread.delete" => {
            ctx.sink.delete(viewer, thread_id(command)?).await?;
        }
        "project.meta.update" => project_meta_update(command)?,
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

    dispatch_result(ctx).await
}

pub(crate) async fn dispatch_result(ctx: &CommandContext<'_>) -> Result<Value, CommandRefusal> {
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
    let text = inline_message_text(
        message
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        message.get("context"),
    );
    let message_uuid = message
        .get("messageId")
        .and_then(Value::as_str)
        .ok_or_else(|| missing("message.messageId"))?
        .to_string();
    let agent_mode = interaction_mode(command);
    let effort = selected_effort(command);
    let attachments = attachments(message)?;
    let queue = command.get("dispatchMode").and_then(Value::as_str) == Some("queue");

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
                        harness: None,
                        agent_mode,
                        effort,
                        attachments,
                        skills,
                        queue,
                    }),
                    provision: false,
                },
            )
            .await;
    }

    let conversation = require_controllable(ctx, viewer, &task_id).await?;
    let agents = ctx.agents.list(viewer).await;
    let Selection { harness, model } = selection_for_existing(&agents, &conversation, command)?;
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
                harness,
                agent_mode,
                effort,
                attachments,
                skills,
                queue,
            },
        )
        .await
}

/// The skills a turn's text activates, as names Moatless frontloads.
///
/// A `$name` mention only activates a skill this Workspace actually offers —
/// checked against the listing — so a `$HOME` in prose activates nothing; a
/// prompt with no mention skips that read.
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

/// The largest single image, decoded (`contracts/src/orchestration.ts:145`).
///
/// A backstop against a client that skips the composer's re-encoding, not a
/// limit a person meets by pasting a screenshot.
const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// `ChatAttachmentId`'s length (`contracts/src/orchestration.ts:175`).
const MAX_ATTACHMENT_ID_CHARS: usize = 128;

/// What the composer sent with a turn: a *reference* names an already-uploaded
/// id, an *inline* one (image only) carries a `data:` URL.
///
/// Its mime type and size are read from the decoded inline payload, not the
/// client-declared fields, and anything invalid is rejected rather than
/// silently dropped.
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
/// `ChatAttachment` is a union of `image`, `file`, and an open catch-all for a
/// future type (refused by name, not guessed); `dataUrl` marks an inline image,
/// everything else names an upload by id.
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
/// Only base64 payloads are read — the one form the composer produces; a
/// browser never builds a percent-encoded `data:` URL from a `File`.
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

/// A thread created empty — from a proposed plan, or a draft the person ran a
/// Script in before sending anything.
///
/// `provision` separates the two: a plan's thread awaits the Sandbox until its
/// first turn; a draft's Script needs one now.
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

/// Branches a new Thread off an existing one at one of its completed turns.
///
/// An `instanceId` naming another harness than the source's runs the fork on
/// it; without one the fork runs the source's profile.
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
    let Selection { harness, model } = selection_for_existing(&agents, &source, command)?;

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
                harness,
                agent_mode: interaction_mode(command),
                effort: selected_effort(command),
            },
        )
        .await
}

/// The title a rename carries; drops it unless the Task already has a name.
///
/// A new Task is unnamed, and the client's auto-title command is
/// indistinguishable on the wire from a person's rename. Letting it through
/// would set `tasks.name` before the pod boots and skip the agent's own
/// naming step (`agent/prompt.rs`'s `# Required First Actions`).
fn rename(conversation: &Conversation, command: &Value) -> Option<String> {
    let title = command.get("title").and_then(Value::as_str)?;
    conversation
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())?;
    Some(title.to_string())
}

/// The source Task's turn a fork cuts after, if the command named one. `None`
/// cuts after the newest turn that reached a terminal status.
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

/// Title, model and harness. `branch`, `expectedBranch` and `worktreePath` are read and
/// ignored: a Task's branch is decided at creation and the checkout lives in
/// the Sandbox.
async fn meta_update(
    ctx: &CommandContext<'_>,
    viewer: &ScopeContext,
    command: &Value,
) -> Result<(), CommandRefusal> {
    let task_id = thread_id(command)?.to_string();
    let conversation = require_controllable(ctx, viewer, &task_id).await?;
    let agents = ctx.agents.list(viewer).await;

    let Selection { harness, model } = selection_for_existing(&agents, &conversation, command)?;

    ctx.sink
        .update_meta(
            viewer,
            ThreadMeta {
                task_id: task_id.clone(),
                title: rename(&conversation, command),
                model,
                harness,
            },
        )
        .await?;
    if command.get("regenerateTitle").and_then(Value::as_bool) == Some(true) {
        regenerate_title(ctx, viewer, &task_id).await?;
    }
    Ok(())
}

/// What a title regeneration asks the agent; its `moat set-name` answers it.
const REGENERATE_TITLE_PROMPT: &str =
    "Please rename this task to reflect the conversation so far, with `moat set-name`.";

/// Ask the agent to rename the Task, the way the git toolbar asks it to open
/// a pull request (`git_actions`). Queued, so work in progress finishes first.
async fn regenerate_title(
    ctx: &CommandContext<'_>,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<(), CommandRefusal> {
    ctx.sink
        .send_turn(
            viewer,
            ThreadTurn {
                task_id: task_id.to_string(),
                text: REGENERATE_TITLE_PROMPT.to_string(),
                message_uuid: Uuid::new_v4().to_string(),
                model: None,
                harness: None,
                agent_mode: None,
                effort: None,
                attachments: Vec::new(),
                skills: Vec::new(),
                queue: true,
            },
        )
        .await
}

/// Refuses every project change: a Workspace is edited only in Git.
///
/// Each `PROJECT_FIELDS_MOATLESS_OWNS` field and a non-null `scripts` is
/// refused by name rather than dropped, so a client never sees a false save;
/// an update carrying none is a no-op.
fn project_meta_update(command: &Value) -> Result<(), CommandRefusal> {
    command
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

    if command.get("scripts").is_none_or(Value::is_null) {
        // The client folds unrelated project state into this command.
        return Ok(());
    }

    Err(CommandRefusal::Rejected {
        message: "Workspaces are managed in Git. Edit `.moatless/workspaces.json` in the \
                  repository to change its scripts."
            .into(),
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

/// What the agent reads when a person closes its question unanswered.
const DISMISSED_QUESTION: &str = "The user dismissed this question without answering.";
const DISMISSED_PLAN: &str = "The user dismissed this plan without a decision.";

/// An answer to what the agent asked.
///
/// `requestId` names the tool-use call that asked, and its stored tool decides
/// the answer's shape: a question takes one answer per question, a proposed
/// plan takes a yes or no — conflating the two would label a plan answer as a
/// question answer.
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

    let dismissed = command.get("dismissed").and_then(Value::as_bool) == Some(true);
    let answers = answers_by_question(command.get("answers"));
    let payload = match call.tool_name.as_str() {
        mapping::QUESTION_TOOL if dismissed => {
            let answers: serde_json::Map<String, Value> = call
                .questions
                .iter()
                .map(|question| (question.clone(), json!(DISMISSED_QUESTION)))
                .collect();
            json!({ "answers": answers })
        }
        mapping::PLAN_TOOL if dismissed => json!({ "approved": false, "feedback": DISMISSED_PLAN }),
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

/// A queued-turn command as the [`QueueChange`] it asks for, on a thread the
/// viewer may run turns on.
async fn change_queue(
    ctx: &CommandContext<'_>,
    viewer: &ScopeContext,
    tag: &str,
    command: &Value,
) -> Result<(), CommandRefusal> {
    let task_id = thread_id(command)?;
    let message_uuid = || {
        command
            .get("messageId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| missing("messageId"))
    };
    let change = match tag {
        "thread.queuedTurn.cancel" => QueueChange::Cancel {
            message_uuid: message_uuid()?,
        },
        "thread.queuedTurn.edit" => QueueChange::Edit {
            message_uuid: message_uuid()?,
            text: inline_message_text(
                command
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or_else(|| missing("text"))?,
                command.get("context"),
            ),
            attachments: match command.get("attachments") {
                Some(Value::Array(_)) => Some(attachments(command)?),
                _ => None,
            },
        },
        "thread.queuedTurn.reorder" => QueueChange::Reorder {
            message_uuid: message_uuid()?,
            before: command
                .get("beforeMessageId")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
        _ => QueueChange::Resume,
    };
    require_controllable(ctx, viewer, task_id).await?;
    ctx.sink.change_queue(viewer, task_id, change).await
}

/// The `ThreadPullRequestKey` a link or unlink command carries, on a Thread
/// this viewer may write to.
///
/// Gated on control rather than read: a link changes what every surface reports
/// for the Thread, and a reader may open one they cannot write.
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

/// A pin command's `orderKey`, trimmed; `None` when absent. A blank key is
/// refused: the contract types it as a non-empty string, and storing one
/// would fail the next listing's decode.
fn optional_order_key(command: &Value) -> Result<Option<String>, CommandRefusal> {
    match command.get("orderKey") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_str()
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .map(|key| Some(key.to_string()))
            .ok_or_else(|| missing("orderKey")),
    }
}

/// One answer per question, keyed on the question's full text.
///
/// A multi-select `string[]` answer is joined into a string here: the CLI
/// merges it into the tool input unread, and the panel already sends a joined
/// string. Anything neither string nor array is dropped, not stringified.
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
/// Only the approve option approves; everything else is a rejection carrying
/// what was said as feedback, so "No, use the second approach" reaches the
/// agent as both the rejection and the reason.
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
/// Gated on control, not just read — a turn runs in the Sandbox a controller
/// can already act in — and reports failure the same way as a Task that doesn't
/// exist. Archived Tasks fail here even for a reader who may still open them;
/// unarchiving is the way back and does not go through this function.
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

/// What a command on an existing Thread asks its agent to run.
struct Selection {
    /// Set when the command's instance names another harness than the Task's.
    harness: Option<Harness>,
    /// A catalog model id of the selected instance.
    model: Option<String>,
}

/// The harness and catalog model a command asks for. A bare model slug with
/// no `instanceId` selects neither: the client sends one it cached from
/// another Thread.
fn selection_for_existing(
    agents: &[crate::agent::schemas::AgentDescriptor],
    conversation: &Conversation,
    command: &Value,
) -> Result<Selection, CommandRefusal> {
    let Some(instance) = selected_instance(command) else {
        return Ok(Selection {
            harness: None,
            model: None,
        });
    };
    let harness = providers::harness_for(&instance).ok_or_else(|| CommandRefusal::Rejected {
        message: format!("`{instance}` is not an agent this server offers."),
    })?;
    let thread_harness =
        conversation
            .agent_profile
            .harness()
            .ok_or_else(|| CommandRefusal::Rejected {
                message: format!(
                    "This thread runs `{}`, an agent this server does not know.",
                    conversation.agent_profile
                ),
            })?;
    Ok(Selection {
        harness: (thread_harness != harness).then_some(harness),
        model: translate_model(agents, &instance, command)?,
    })
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
/// The contract sends `null` for "the repository's default", reaching creation
/// as `None`; a blank string carries the same intent and is read the same way
/// rather than pinned as a branch named "".
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
/// T3 carries every per-model choice as `{id, value}` selections on
/// `modelSelection.options`, keyed by the descriptor ids this provider
/// published (`providers::effort_option_id`); a selection this server never
/// published is ignored rather than forwarded to an agent with no such setting.
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
mod tests;
