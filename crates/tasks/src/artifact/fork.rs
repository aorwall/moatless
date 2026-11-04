//! Forking an Artifact version: a copy that starts a private Artifact of its
//! own on another Task, and the Task it lands in.
//!
//! The Task is created idle and is sent its first message only once the copy
//! is committed, so its agent cannot look for the copy before it exists.

use std::future::Future;
use std::sync::Arc;

use moatless_api_schemas::artifact::{ArtifactForkMode, ForkArtifactRequest};
use moatless_api_schemas::task::CreateTaskRequest;
use moatless_types::access_control::ScopeContext;
use moatless_types::task::ExternalOrigin;

use super::error::ArtifactError;
use super::models::{Artifact, DataAccess, NewFork};
use super::operations::Artifacts;
use super::schemas::ArtifactResponse;
use crate::message::send::MessageSendService;
use crate::task::creation::TaskCreationService;
use crate::task::error::TaskError;
use crate::task::fork::{
    FORK_NAME_PREFIX, ForkCreationPorts, ForkProfile, ForkRequest, TaskForkService,
    fork_first_message,
};
use crate::task::models::Task;
use crate::task::service::{self, CreateTaskIntent, CreateTaskPorts};

/// Longest reader message a fork's first message carries.
const MAX_MESSAGE_LEN: usize = 20_000;

/// Everything a fork creates its Task through.
pub struct ArtifactForkPorts<'a> {
    pub create: ForkCreationPorts<'a>,
    pub task_fork: &'a TaskForkService,
}

/// A fork's new Task and the copy on it.
#[derive(Debug)]
pub struct ArtifactForkOutcome {
    pub task: Task,
    pub artifact: ArtifactResponse,
}

/// Fork `version` of `artifact_id` into the Task `request.mode` names, as
/// `scope`'s reader. `Task` mode answers not found to a reader who cannot read
/// the Artifact's Task, as every other route does.
pub async fn fork<EvFut>(
    ports: ArtifactForkPorts<'_>,
    artifacts: &Artifacts,
    scope: &ScopeContext,
    artifact_id: &str,
    version: &str,
    request: ForkArtifactRequest,
    build_event_payload: impl FnOnce(Task) -> EvFut + Send,
) -> Result<ArtifactForkOutcome, ArtifactError>
where
    EvFut: Future<Output = serde_json::Value> + Send,
{
    let message = request
        .message
        .as_deref()
        .map(str::trim)
        .filter(|message| !message.is_empty());
    if message.is_some_and(|message| message.chars().count() > MAX_MESSAGE_LEN) {
        return Err(ArtifactError::BadRequest(format!(
            "message must be at most {MAX_MESSAGE_LEN} characters"
        )));
    }
    if !artifacts.is_configured() {
        return Err(ArtifactError::StorageNotConfigured);
    }
    let tasks = ports.create.normalize.tasks;
    let (row, version) =
        super::service::find_version(tasks, artifacts, scope, artifact_id, Some(version)).await?;
    let html = artifacts
        .read_page(&row.task_id, &row.artifact_id, version)
        .await?
        .ok_or_else(|| ArtifactError::NotFound(format!("{artifact_id}/v{version}")))?;

    let message_send = ports.create.message_send.clone();
    // `Some` for a fork of the source Task, holding its cut.
    let (task, fork_cut) = match request.mode {
        ArtifactForkMode::Task => {
            let at_turn = artifacts
                .list_versions(&row.artifact_id)
                .await?
                .into_iter()
                .find(|recorded| recorded.version == version)
                .and_then(|recorded| recorded.turn_number);
            let fork = ForkRequest {
                side_chat: false,
                source_task_id: row.task_id.clone(),
                new_task_id: None,
                at_turn,
                message: None,
                name: None,
                same_sandbox: false,
                profile: ForkProfile::Source,
                model: None,
                agent_mode: None,
                effort: None,
                skills: Vec::new(),
            };
            let outcome = ports
                .task_fork
                .fork(ports.create, scope, fork, build_event_payload)
                .await
                .map_err(TaskError::from)?;
            (outcome.task, Some(at_turn))
        }
        ArtifactForkMode::New => {
            let workspace_id = request
                .workspace_id
                .filter(|id| !id.trim().is_empty())
                .ok_or_else(|| {
                    ArtifactError::BadRequest("a new Task needs a workspaceId".to_string())
                })?;
            let task = create_task(
                ports.create.normalize,
                ports.create.task_creation,
                ports.create.message_send,
                scope,
                workspace_id,
                &row.title,
                build_event_payload,
            )
            .await?;
            (task, None)
        }
    };

    let copy = copy_into(artifacts, scope, &row, version, &task.task_id, html).await?;
    let text = brief(&row, version, &copy, message, fork_cut.is_some());
    let text = match fork_cut {
        Some(at_turn) => fork_first_message(&row.task_id, at_turn, &text),
        None => text,
    };
    message_send
        .send_message_flow(
            &task.task_id,
            &text,
            None,
            None,
            None,
            &[],
            &[],
            true,
            ExternalOrigin::none(),
            false,
            Some(scope.user_id.as_str()),
            &[],
            None,
            false,
            None,
            None,
        )
        .await
        .map_err(TaskError::FirstMessage)?;
    tracing::info!(
        task_id = %task.task_id,
        source_artifact_id = %row.artifact_id,
        source_version = version,
        artifact_id = %copy.artifact_id,
        "Artifact forked"
    );
    Ok(ArtifactForkOutcome {
        task,
        artifact: copy,
    })
}

/// Create the idle Task a `new` fork lands in, in `workspace_id`.
async fn create_task<EvFut>(
    normalize: CreateTaskPorts<'_>,
    task_creation: &TaskCreationService,
    message_send: Arc<MessageSendService>,
    scope: &ScopeContext,
    workspace_id: String,
    title: &str,
    build_event_payload: impl FnOnce(Task) -> EvFut + Send,
) -> Result<Task, ArtifactError>
where
    EvFut: Future<Output = serde_json::Value> + Send,
{
    let request = CreateTaskRequest {
        repository_id: None,
        workspace_id: Some(workspace_id),
        sandbox_id: None,
        checkout_mode: None,
        agent_profile: None,
        name: Some(format!("{FORK_NAME_PREFIX}{title}")),
        description: None,
        branch: None,
        message: None,
        model: None,
        agent_mode: None,
        effort: None,
        contexts: Vec::new(),
        file_ids: Vec::new(),
        parent_task_id: None,
        run_as_user_id: None,
        headless: None,
        skills: Vec::new(),
        agent_properties: Default::default(),
    };
    let new_task = service::normalize(
        normalize,
        scope,
        &scope.user_id,
        CreateTaskIntent::new(request),
    )
    .await?;
    let outcome =
        service::create_task(task_creation, message_send, new_task, build_event_payload).await?;
    Ok(outcome.task)
}

/// Copy `version` of `source` onto `task_id` as version 1 of a new private
/// Artifact under the same slug. The copy keeps the page's record settings
/// but none of its records, and drops `notify` and attached collections:
/// both belong to the source Task's agent and scopes.
async fn copy_into(
    artifacts: &Artifacts,
    scope: &ScopeContext,
    source: &Artifact,
    version: i32,
    task_id: &str,
    html: Vec<u8>,
) -> Result<ArtifactResponse, ArtifactError> {
    let fork = NewFork {
        artifact_id: uuid::Uuid::new_v4().to_string(),
        task_id: task_id.to_string(),
        slug: source.slug.clone(),
        title: source.title.clone(),
        created_by: scope.user_id.clone(),
        data: DataAccess {
            accepts_data: source.accepts_data,
            notify_on_data: false,
            rules: source.record_rules.clone(),
            collections: Vec::new(),
        },
        source_artifact_id: source.artifact_id.clone(),
        source_version: version,
    };
    artifacts
        .put_page(task_id, &fork.artifact_id, 1, html)
        .await?;
    match artifacts.insert_fork(&fork).await {
        Ok(row) => Ok(super::service::to_response(
            &row,
            artifacts.public_url(),
            1,
            &row.title,
        )),
        Err(error) => {
            artifacts.delete_page(task_id, &fork.artifact_id, 1).await;
            Err(error.into())
        }
    }
}

/// The first message that hands a fork's agent its copy. `in_fork` is a fork
/// of the source Task, whose agent already knows the page.
fn brief(
    source: &Artifact,
    version: i32,
    copy: &ArtifactResponse,
    message: Option<&str>,
    in_fork: bool,
) -> String {
    let origin = if in_fork {
        format!(
            "Your Artifact `{}` (v{version}) was forked into this Task",
            source.slug
        )
    } else {
        format!(
            "The Artifact \"{}\" (v{version}) from another Task was forked into this Task",
            source.title
        )
    };
    let mut flags = String::new();
    if copy.accepts_data {
        flags.push_str(" --data");
    }
    if copy.record_rules.is_some() {
        // The rules `moat artifact list` shows; a publish without them drops them.
        flags.push_str(" --rules <rules.json>");
    }
    let request = message.unwrap_or("Fetch the page and ask what to change.");
    format!(
        "{origin} as version 1 of its own private Artifact: {url}\n\n\
         Fetch it with `moat artifact get {id} -o {slug}.html`. Publish each change with \
         `moat artifact publish {slug}.html --title {title} --slug {slug}{flags}`, which \
         makes the next version of the copy; the original is not changed.\n\n{request}",
        url = copy.url,
        id = copy.artifact_id,
        slug = copy.slug,
        title = shell_quote(&copy.title),
    )
}

/// `text` as one single-quoted POSIX shell word.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
#[path = "fork_tests.rs"]
mod tests;
