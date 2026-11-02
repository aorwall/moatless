//! Orchestration V2's command union, rewritten onto the V1 commands
//! [`commands::dispatch`] already carries out, so a V2 client reaches
//! the same Moatless writes a V1 client does.
//!
//! Three outcomes per command: translated, discarded (the client keeps the
//! state locally and nothing here records it), or refused with a reason.

use serde_json::{Map, Value, json};

use moatless_types::access_control::ScopeContext;

use crate::commands;
use crate::commands::CommandContext;
use crate::ports::CommandRefusal;
use crate::projection::queued_message_uuid;
use crate::projection::turn_count_of_checkpoint;
use crate::projection::turn_number_of_run;

/// Answers one orchestration V2 command by way of the V1 command it
/// [translates](translate) to.
pub(crate) async fn dispatch(
    ctx: &CommandContext<'_>,
    viewer: &ScopeContext,
    command: &Value,
) -> Result<Value, CommandRefusal> {
    match translate(command)? {
        Translated::V1(v1) => commands::dispatch(ctx, viewer, &v1).await,
        Translated::Sequence(steps) => {
            let mut result = None;
            for step in &steps {
                result = Some(commands::dispatch(ctx, viewer, step).await?);
            }
            match result {
                Some(result) => Ok(result),
                None => commands::dispatch_result(ctx).await,
            }
        }
        Translated::Discarded => commands::dispatch_result(ctx).await,
    }
}

/// What one V2 command becomes.
#[derive(Debug, PartialEq)]
pub(crate) enum Translated {
    /// The V1 command that carries it out.
    V1(Value),
    /// V1 commands carried out in order. The first refusal stops the rest and
    /// answers the whole command, though the steps before it have taken effect.
    Sequence(Vec<Value>),
    /// Accepted and dropped: nothing in Moatless records it.
    Discarded,
}

/// Rewrite one V2 command, or refuse it.
pub(crate) fn translate(command: &Value) -> Result<Translated, CommandRefusal> {
    let tag = command
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let renamed = |v1: &str| Ok(Translated::V1(with_type(command, v1)));
    match tag {
        // Same name and the same fields on both sides.
        "thread.create"
        | "thread.archive"
        | "thread.unarchive"
        | "thread.delete"
        | "thread.settle"
        | "thread.unsettle"
        | "thread.snooze"
        | "thread.unsnooze"
        | "thread.auto-settle.set"
        | "thread.pin"
        | "thread.unpin"
        | "thread.pin.reorder"
        | "thread.active.reorder"
        | "thread.follow"
        | "thread.unfollow"
        | "thread.visibility.set"
        | "thread.pull-request.link"
        | "thread.pull-request.unlink" => renamed(tag),
        "thread.metadata.update" | "thread.model-selection.set" => renamed("thread.meta.update"),
        "run.interrupt" => renamed("thread.turn.interrupt"),
        "provider-session.detach" => renamed("thread.session.stop"),
        "message.dispatch" => message_dispatch(command),
        "queued-message.promote-to-steer" => {
            let queued = command
                .get("queuedRunId")
                .and_then(Value::as_str)
                .ok_or_else(|| missing("queuedRunId"))?;
            let message_uuid =
                queued_message_uuid(queued).ok_or_else(|| CommandRefusal::Rejected {
                    message: format!("`{queued}` is not a queued message on this server."),
                })?;
            Ok(Translated::V1(json!({
                "type": "thread.queuedTurn.steer",
                "threadId": command.get("threadId"),
                "messageId": message_uuid,
            })))
        }
        "runtime-request.respond" => {
            if command
                .get("decision")
                .is_some_and(|value| !value.is_null())
            {
                return refused(tag, "Approvals are not part of this server.");
            }
            renamed("thread.user-input.respond")
        }
        "checkpoint.rollback" => checkpoint_rollback(command),
        "thread.fork" => fork(command),
        "thread.visit" | "thread.mark-unread" => renamed(tag),
        "queued-run.cancel" | "queued-run.edit" | "queued-run.reorder" => queued_run(command),
        "queue.resume" => renamed("thread.queue.resume"),
        "thread.user-input.dismiss" => {
            let mut v1 = Map::new();
            v1.insert("type".into(), json!("thread.user-input.respond"));
            copy(command, &mut v1, &["commandId", "threadId", "requestId"]);
            v1.insert("dismissed".into(), json!(true));
            Ok(Translated::V1(Value::Object(v1)))
        }
        "thread.pull-request.watch" => Ok(pull_request_watch(command)),
        "thread.title.regeneration.complete" => refused(
            tag,
            "The agent's own rename completes a title regeneration on this server.",
        ),
        // Kept by the client itself; Moatless has no column for either.
        "thread.runtime-mode.set" | "thread.interaction-mode.set" => Ok(Translated::Discarded),
        "thread.merge_back" => merge_back(command),
        other => refused(other, "This server does not implement that action."),
    }
}

/// A queued-run command names its turn by the run id the projection minted for
/// it; V1 names the message that run waits to deliver.
fn queued_run(command: &Value) -> Result<Translated, CommandRefusal> {
    let tag = command
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let message_of = |field: &str| -> Result<Value, CommandRefusal> {
        let run_id = command
            .get(field)
            .and_then(Value::as_str)
            .ok_or_else(|| missing(field))?;
        queued_message_uuid(run_id)
            .map(|uuid| json!(uuid))
            .ok_or_else(|| CommandRefusal::Rejected {
                message: format!("`{run_id}` is not a queued message on this server."),
            })
    };
    let mut v1 = Map::new();
    copy(command, &mut v1, &["commandId", "threadId"]);
    v1.insert("messageId".into(), message_of("runId")?);
    let v1_tag = match tag {
        "queued-run.cancel" => "thread.queuedTurn.cancel",
        "queued-run.edit" => {
            copy(command, &mut v1, &["text", "context", "attachments"]);
            "thread.queuedTurn.edit"
        }
        _ => {
            let before = match command.get("beforeRunId") {
                None | Some(Value::Null) => Value::Null,
                Some(_) => message_of("beforeRunId")?,
            };
            v1.insert("beforeMessageId".into(), before);
            "thread.queuedTurn.reorder"
        }
    };
    v1.insert("type".into(), json!(v1_tag));
    Ok(Translated::V1(Value::Object(v1)))
}

/// A rollback to one of the projection's checkpoints is V1's conversation
/// revert to that turn count. Asking for files back goes to V1's checkpoint
/// revert, which refuses it: Moatless takes no worktree snapshots.
fn checkpoint_rollback(command: &Value) -> Result<Translated, CommandRefusal> {
    let checkpoint = command
        .get("checkpointId")
        .and_then(Value::as_str)
        .ok_or_else(|| missing("checkpointId"))?;
    let turn_count =
        turn_count_of_checkpoint(checkpoint).ok_or_else(|| CommandRefusal::Rejected {
            message: format!("`{checkpoint}` is not a checkpoint on this server."),
        })?;
    let restore_files = command.get("restoreFiles").and_then(Value::as_bool) == Some(true);
    let tag = if restore_files {
        "thread.checkpoint.revert"
    } else {
        "thread.conversation.revert"
    };
    Ok(Translated::V1(json!({
        "type": tag,
        "commandId": command.get("commandId"),
        "threadId": command.get("threadId"),
        "turnCount": turn_count,
    })))
}

/// `message.dispatch` is V1's `thread.turn.start` without the bootstrap. Any
/// message steers a running turn, so `start_immediately` and `steer_active`
/// are a plain send. A restart interrupts the thread's open turn, whatever
/// `targetRunId` names, then queues the message so it waits for that turn to
/// end. Deferred start has no Moatless counterpart.
fn message_dispatch(command: &Value) -> Result<Translated, CommandRefusal> {
    let tag = "message.dispatch";
    let mode = command
        .get("dispatchMode")
        .and_then(|mode| mode.get("type"))
        .and_then(Value::as_str)
        .unwrap_or("start_immediately");
    let restart = mode == "restart_active"
        || command.get("deliveryIntent").and_then(Value::as_str) == Some("restart");
    let queue = match mode {
        "start_immediately" | "steer_active" => restart,
        "queue_after_active" | "restart_active" => true,
        _ => return refused(tag, "Deferred starts are not part of this server."),
    };
    let mut v1 = Map::new();
    v1.insert("type".into(), json!("thread.turn.start"));
    copy(
        command,
        &mut v1,
        &["commandId", "threadId", "modelSelection"],
    );
    v1.insert(
        "message".into(),
        json!({
            "messageId": command.get("messageId").ok_or_else(|| missing("messageId"))?,
            "text": command.get("text").cloned().unwrap_or_else(|| json!("")),
            "context": command.get("context"),
            "attachments": command.get("attachments").cloned().unwrap_or_else(|| json!([])),
        }),
    );
    if queue {
        v1.insert("dispatchMode".into(), json!("queue"));
    }
    if restart {
        // Waiting relies on the pod holding queued messages; one that does not
        // ask to hold them (`inbox::models`) steers this into the dying turn.
        let interrupt = json!({
            "type": "thread.turn.interrupt",
            "threadId": command.get("threadId"),
        });
        return Ok(Translated::Sequence(vec![interrupt, Value::Object(v1)]));
    }
    Ok(Translated::V1(Value::Object(v1)))
}

/// A watch that asks to link the pull request first is the link and then the
/// watch, so a refused link leaves the watch unset.
fn pull_request_watch(command: &Value) -> Translated {
    let watch = command.clone();
    let link = command.get("link").filter(|link| !link.is_null());
    let starting = command.get("watching").and_then(Value::as_bool) == Some(true);
    let Some(link) = link.filter(|_| starting) else {
        return Translated::V1(watch);
    };
    let mut v1 = Map::new();
    v1.insert("type".into(), json!("thread.pull-request.link"));
    copy(
        command,
        &mut v1,
        &["commandId", "threadId", "host", "repository", "number"],
    );
    copy(link, &mut v1, &["url", "source"]);
    Translated::Sequence(vec![Value::Object(v1), watch])
}

fn merge_back(command: &Value) -> Result<Translated, CommandRefusal> {
    let source = command
        .get("sourceThreadId")
        .and_then(Value::as_str)
        .ok_or_else(|| missing("sourceThreadId"))?;
    let point = command
        .get("sourcePoint")
        .ok_or_else(|| missing("sourcePoint"))?;
    let run = point
        .get("runId")
        .and_then(Value::as_str)
        .ok_or_else(|| missing("sourcePoint.runId"))?;
    let through = turn_number_of_run(run)
        .filter(|_| {
            point.get("type").and_then(Value::as_str) == Some("run")
                && run.rsplit_once(':').is_some_and(|(task, _)| task == source)
        })
        .ok_or_else(|| CommandRefusal::Rejected {
            message: "Choose a completed run of the source thread.".into(),
        })?;
    let mut v1 = Map::new();
    copy(
        command,
        &mut v1,
        &["commandId", "sourceThreadId", "targetThreadId"],
    );
    v1.insert("type".into(), json!("thread.merge_back"));
    v1.insert("atTurn".into(), json!(through));
    Ok(Translated::V1(Value::Object(v1)))
}

/// V2 names the fork's new thread `targetThreadId` and its cut point as a
/// `sourcePoint`; V1 takes `threadId` and an `atTurn` turn number.
fn fork(command: &Value) -> Result<Translated, CommandRefusal> {
    let tag = "thread.fork";
    let point = command.get("sourcePoint");
    let at_turn = match point
        .and_then(|point| point.get("type"))
        .and_then(Value::as_str)
    {
        None | Some("latest_stable") => Value::Null,
        Some("run") => {
            let run_id = point
                .and_then(|point| point.get("runId"))
                .and_then(Value::as_str)
                .ok_or_else(|| missing("sourcePoint.runId"))?;
            json!(
                turn_number_of_run(run_id).ok_or_else(|| CommandRefusal::Rejected {
                    message: format!("`{run_id}` is not a completed turn to fork from."),
                })?
            )
        }
        Some(_) => return refused(tag, "This server takes no checkpoints to fork from."),
    };
    let mut v1 = Map::new();
    v1.insert("type".into(), json!(tag));
    copy(
        command,
        &mut v1,
        &[
            "commandId",
            "sourceThreadId",
            "sideChat",
            "title",
            "interactionMode",
            "sameSandbox",
            "message",
            "modelSelection",
        ],
    );
    v1.insert(
        "threadId".into(),
        command
            .get("targetThreadId")
            .cloned()
            .ok_or_else(|| missing("targetThreadId"))?,
    );
    v1.insert("atTurn".into(), at_turn);
    Ok(Translated::V1(Value::Object(v1)))
}

/// A `projects.mutate` mutation as the V1 command that carries it out. An update
/// is `project.meta.update`, which writes `scripts` and refuses the fields a
/// Workspace owns; `project.create` and `project.delete` are refused there too.
pub(crate) fn project_mutation(mutation: &Value) -> Result<Value, CommandRefusal> {
    match mutation.get("type").and_then(Value::as_str) {
        Some("project.update") => {
            // Not among V1's refused fields: no V1 command carries it.
            if mutation
                .get("autoPull")
                .is_some_and(|value| !value.is_null())
            {
                return Err(CommandRefusal::Unsupported {
                    command: "project.update (autoPull)".to_string(),
                    reason: "A project is a Moatless workspace; change it there.".to_string(),
                });
            }
            Ok(with_type(mutation, "project.meta.update"))
        }
        _ => Ok(mutation.clone()),
    }
}

fn with_type(command: &Value, v1_type: &str) -> Value {
    let mut v1 = command.as_object().cloned().unwrap_or_default();
    v1.insert("type".into(), json!(v1_type));
    Value::Object(v1)
}

fn copy(from: &Value, to: &mut Map<String, Value>, keys: &[&str]) {
    for key in keys {
        if let Some(value) = from.get(*key) {
            to.insert((*key).to_string(), value.clone());
        }
    }
}

fn refused(command: &str, reason: &str) -> Result<Translated, CommandRefusal> {
    Err(CommandRefusal::Unsupported {
        command: command.to_string(),
        reason: reason.to_string(),
    })
}

fn missing(field: &str) -> CommandRefusal {
    CommandRefusal::Rejected {
        message: format!("The command is missing `{field}`."),
    }
}

#[cfg(test)]
mod tests;
