//! The `git.*` write methods: `git.runStackedAction` and the pull-request
//! dialog. This module composes nothing and sequences nothing — it turns one
//! `cwd` into one Task, refuses what the viewer may not reach, and relays the
//! Sandbox's own event stream onto the socket. The phases, the hooks and the
//! pull request all run in the Sandbox; here they are only forwarded.

use futures::StreamExt;
use moatless_backend_core::sandbox::{SandboxConnector, classify_grpc_error};
use moatless_backend_core::scope::ScopeContext;
use sandbox_proto::git_action_event::Event as ProtoEvent;
use sandbox_proto::{
    GitActionResult, GitBranchStep, GitCommitStep, GitPrStep, GitPushStep, GitStackedAction,
    RunGitStackedActionRequest,
};
use serde_json::{Value, json};
use tonic::Streaming;

use crate::dao::ConversationDao;
use crate::data::T3WorkspaceDao;
use crate::ports::{BoundPullRequestSource, PullRequestError, ResolvedPullRequest};
use crate::projection;
use crate::workspace::{self, WorkspaceAccessError};

/// The identifiers every event on one action's stream carries, echoed back so
/// the client pairs the stream with the button it pressed.
pub struct ActionContext {
    pub action_id: String,
    pub cwd: String,
    pub action: String,
}

/// An opened action: the Sandbox's event stream, the context to stamp on every
/// frame relayed from it, and the Task it runs against so a branch it creates
/// can be recorded back onto the Task.
pub struct OpenedAction {
    pub stream: Streaming<sandbox_proto::GitActionEvent>,
    pub context: ActionContext,
    pub task_id: String,
}

/// Why an action could not be opened. A refusal the viewer caused (an
/// unaddressable `cwd`, a stopped sandbox, a malformed request) rather than a
/// failure part-way through — those arrive as `action_failed` on the stream.
pub enum OpenError {
    Access(WorkspaceAccessError),
    Rejected(String),
}

/// Resolve the `cwd`, refuse what the viewer cannot reach, and open the
/// Sandbox's action stream. Nothing is run here; the first phase begins when the
/// Sandbox produces its first event.
pub async fn open_stacked_action(
    conversations: &dyn ConversationDao,
    sandboxes: &dyn SandboxConnector,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<OpenedAction, OpenError> {
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
        .ok_or_else(|| OpenError::Rejected("cwd is required".to_string()))?
        .to_string();
    let action_id = payload
        .get("actionId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| OpenError::Rejected("actionId is required".to_string()))?
        .to_string();
    let action = payload
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| OpenError::Rejected("action is required".to_string()))?
        .to_string();
    let proto_action = proto_action(&action)
        .ok_or_else(|| OpenError::Rejected(format!("unknown action `{action}`")))?;

    let commit_message = payload
        .get("commitMessage")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .map(str::to_string);
    let feature_branch = payload
        .get("featureBranch")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let file_paths = payload
        .get("filePaths")
        .and_then(Value::as_array)
        .map(|paths| {
            paths
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let task_id = workspace::addressed_conversation(conversations, viewer, &cwd)
        .await
        .map_err(OpenError::Access)?;
    // This method mutates the checkout and can push and open PRs, so following
    // and reading is not enough: the viewer must control the Task. A reader who
    // does not is answered as if the workspace were not there.
    if !conversations
        .controls_conversation(viewer, &task_id)
        .await
        .map_err(|e| OpenError::Access(WorkspaceAccessError::Database(e)))?
    {
        return Err(OpenError::Access(WorkspaceAccessError::Unaddressable));
    }
    let client = sandboxes
        .sandbox_client(&task_id)
        .await
        .map_err(|e| OpenError::Access(workspace::sandbox_access(e)))?;

    let request = RunGitStackedActionRequest {
        action_id: action_id.clone(),
        action: proto_action as i32,
        commit_message,
        feature_branch,
        file_paths,
        repository: None,
    };
    let stream = client
        .run_git_stacked_action(request)
        .await
        .map_err(|status| {
            OpenError::Access(workspace::sandbox_access(classify_grpc_error(
                status,
                "run_git_stacked_action",
            )))
        })?;

    Ok(OpenedAction {
        stream,
        context: ActionContext {
            action_id,
            cwd,
            action,
        },
        task_id,
    })
}

fn proto_action(action: &str) -> Option<GitStackedAction> {
    match action {
        "commit" => Some(GitStackedAction::Commit),
        "push" => Some(GitStackedAction::Push),
        "create_pr" => Some(GitStackedAction::CreatePr),
        "commit_push" => Some(GitStackedAction::CommitPush),
        "commit_push_pr" => Some(GitStackedAction::CommitPushPr),
        _ => None,
    }
}

/// The outcome of preparing a PR-thread: either the local-mode result the client
/// creates the draft thread from, or a refusal of worktree mode by name.
pub enum PreparedThread {
    Local(Value),
    WorktreeUnsupported,
}

/// Resolve a pull-request reference for the attach-PR dialog, against the primary
/// repository of the Workspace the `cwd` names. Answers T3's
/// `GitResolvePullRequestResult` (`{pullRequest}`).
pub async fn resolve_pull_request(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    pull_requests: &dyn BoundPullRequestSource,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<Value, PullRequestError> {
    let (cwd, reference) = pull_request_ref_input(payload)?;
    let pr = resolve_reference(
        workspaces,
        conversations,
        pull_requests,
        viewer,
        &cwd,
        &reference,
    )
    .await?;
    Ok(json!({ "pullRequest": resolved_pull_request_value(&pr) }))
}

/// Prepare a PR-thread. Local mode resolves the reference and answers the head
/// branch the client creates the draft thread on, with no worktree. Worktree
/// mode is refused by name — this surface materializes nothing on disk here — so
/// each mode is answered explicitly rather than one falling through the other.
pub async fn prepare_pull_request_thread(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    pull_requests: &dyn BoundPullRequestSource,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<PreparedThread, PullRequestError> {
    let mode = payload
        .get("mode")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default();
    match mode {
        "worktree" => Ok(PreparedThread::WorktreeUnsupported),
        "local" => {
            let (cwd, reference) = pull_request_ref_input(payload)?;
            let pr = resolve_reference(
                workspaces,
                conversations,
                pull_requests,
                viewer,
                &cwd,
                &reference,
            )
            .await?;
            Ok(PreparedThread::Local(json!({
                "pullRequest": resolved_pull_request_value(&pr),
                "branch": pr.head_branch,
                // Local mode creates the draft thread on the existing branch;
                // no worktree is cut, so the contract's nullable path is null.
                "worktreePath": Value::Null,
            })))
        }
        _ => Err(PullRequestError::Unavailable(format!(
            "Unknown prepare mode `{mode}`."
        ))),
    }
}

/// The `cwd` and normalized reference an attach-PR request carries.
fn pull_request_ref_input(payload: &Value) -> Result<(String, String), PullRequestError> {
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
        .ok_or_else(|| PullRequestError::Unavailable("cwd is required".to_string()))?
        .to_string();
    let reference = payload
        .get("reference")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|reference| !reference.is_empty())
        .ok_or_else(|| PullRequestError::Unavailable("reference is required".to_string()))?;
    Ok((cwd, normalize_reference(reference)))
}

/// `#123` names pull request 123; every other reference is passed through for the
/// backend to resolve as a bare number or an `owner/repo#N`.
fn normalize_reference(reference: &str) -> String {
    reference
        .strip_prefix('#')
        .filter(|rest| !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit()))
        .map_or_else(|| reference.to_string(), str::to_string)
}

/// Resolve the primary repository the `cwd` names and ask the port to resolve
/// the reference against it.
async fn resolve_reference(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    pull_requests: &dyn BoundPullRequestSource,
    viewer: &ScopeContext,
    cwd: &str,
    reference: &str,
) -> Result<ResolvedPullRequest, PullRequestError> {
    let repository_id = repository_for_cwd(workspaces, conversations, viewer, cwd).await?;
    pull_requests
        .resolve_pull_request(viewer, &repository_id, reference)
        .await
}

/// The primary repository of the Workspace a `cwd` names — a draft thread's
/// project root, or an existing conversation's worktree.
async fn repository_for_cwd(
    workspaces: &dyn T3WorkspaceDao,
    conversations: &dyn ConversationDao,
    viewer: &ScopeContext,
    cwd: &str,
) -> Result<String, PullRequestError> {
    let workspace_id = if let Some(id) = projection::workspace_id_from_project_root(cwd) {
        id.to_string()
    } else if let Some(task_id) = projection::task_id_from_cwd(cwd) {
        conversations
            .list_followed_by_ids(viewer, &[task_id.to_string()])
            .await
            .map_err(|error| PullRequestError::Failed(error.to_string()))?
            .into_iter()
            .next()
            .and_then(|conversation| conversation.workspace_id)
            .ok_or(PullRequestError::NotFound)?
    } else {
        return Err(PullRequestError::NotFound);
    };
    workspaces
        .primary_repository(viewer, &workspace_id)
        .await
        .map_err(|error| PullRequestError::Failed(error.to_string()))?
        .ok_or(PullRequestError::NotFound)
}

fn resolved_pull_request_value(pr: &ResolvedPullRequest) -> Value {
    json!({
        "number": pr.number,
        "title": pr.title,
        "url": pr.url,
        "baseBranch": pr.base_branch,
        "headBranch": pr.head_branch,
        "state": pr.state,
    })
}

/// What this deployment can do and who the viewer is signed in as, for
/// `server.discoverSourceControl`. Environment-scoped and per viewer: git is
/// always present in a Sandbox, GitHub is the one provider wired, and the
/// viewer's GitHub auth is their connected credential — `Some` when they hold a
/// usable one, `None` when they do not — read from their provider-token row, not
/// from how this session authenticated. `Option` fields carry the Effect wire
/// form (`{_tag: "None"}` / `{_tag: "Some", value}`) the contract's
/// `Schema.Option` decodes.
pub fn discovery(github: Option<&crate::ports::GithubIdentity>) -> Value {
    json!({
        "versionControlSystems": [{
            "kind": "git",
            "implemented": true,
            "label": "Git",
            "status": "available",
            "version": option_none(),
            "installHint": "Git is bundled with every Moatless sandbox.",
            "detail": option_none(),
        }],
        "sourceControlProviders": [{
            "kind": "github",
            "label": "GitHub",
            "status": "available",
            "version": option_none(),
            "installHint": "Connect a GitHub account to open pull requests.",
            "detail": option_none(),
            "auth": github_auth(github),
        }],
    })
}

fn github_auth(github: Option<&crate::ports::GithubIdentity>) -> Value {
    match github {
        Some(identity) => {
            let account = match identity.account.as_deref() {
                Some(login) if !login.is_empty() => option_some(json!(login)),
                _ => option_none(),
            };
            json!({
                "status": "authenticated",
                "account": account,
                "host": option_some(json!("github.com")),
                "detail": option_none(),
            })
        }
        None => json!({
            "status": "unauthenticated",
            "account": option_none(),
            "host": option_some(json!("github.com")),
            "detail": option_none(),
        }),
    }
}

fn option_none() -> Value {
    json!({ "_tag": "None" })
}

fn option_some(value: Value) -> Value {
    json!({ "_tag": "Some", "value": value })
}

/// Relay every event the Sandbox produces onto the socket as a `Chunk`, ending
/// when the action finishes, the stream errors, or the client goes away.
///
/// Returns the branch the action created, when it created one, so the caller can
/// move the Task's recorded branch onto it — the checkout moved to `t3/<slug>`
/// but the Task row still names the branch it was created on.
pub async fn forward_action_stream<S>(
    mut stream: S,
    context: ActionContext,
    request_id: super::RequestId,
    frames: tokio::sync::mpsc::Sender<Value>,
) -> Option<String>
where
    S: futures::Stream<Item = Result<sandbox_proto::GitActionEvent, tonic::Status>> + Unpin,
{
    let mut created_branch = None;
    while let Some(next) = stream.next().await {
        let event = match next {
            Ok(event) => event,
            Err(error) => {
                tracing::warn!(%error, "git action stream ended with a sandbox error");
                let _ = frames
                    .send(super::failure_exit(
                        &request_id,
                        "The git action stream ended unexpectedly",
                    ))
                    .await;
                return created_branch;
            }
        };
        // The finished result names the branch the branch phase created; that is
        // the metadata drift the caller closes.
        if let Some(ProtoEvent::ActionFinished(finished)) = &event.event {
            created_branch = finished
                .result
                .as_ref()
                .and_then(|result| result.branch.as_ref())
                .filter(|branch| branch.status == "created")
                .and_then(|branch| branch.name.clone());
        }
        let Some(value) = action_event(&context, event) else {
            continue;
        };
        let chunk = json!({
            "_tag": "Chunk",
            "requestId": request_id.wire(),
            "values": [value],
        });
        if frames.send(chunk).await.is_err() {
            return created_branch;
        }
    }
    created_branch
}

/// Translate one Sandbox proto event into one contract `GitActionProgressEvent`,
/// stamping the action's identifiers on it. The toast the contract wants on the
/// final result is presentation the Sandbox does not mint — it is derived here.
fn action_event(context: &ActionContext, event: sandbox_proto::GitActionEvent) -> Option<Value> {
    let base = |kind: &str| {
        json!({
            "actionId": context.action_id,
            "cwd": context.cwd,
            "action": context.action,
            "kind": kind,
        })
    };
    let value = match event.event? {
        ProtoEvent::ActionStarted(started) => {
            let phases: Vec<&str> = started
                .phases
                .iter()
                .filter_map(|p| phase_name(*p))
                .collect();
            let mut v = base("action_started");
            v["phases"] = json!(phases);
            v
        }
        ProtoEvent::PhaseStarted(phase) => {
            let mut v = base("phase_started");
            v["phase"] = json!(phase_name(phase.phase));
            v["label"] = json!(phase.label);
            v
        }
        ProtoEvent::HookStarted(hook) => {
            let mut v = base("hook_started");
            v["hookName"] = json!(hook.hook_name);
            v
        }
        ProtoEvent::HookOutput(output) => {
            let mut v = base("hook_output");
            v["hookName"] = json!(output.hook_name);
            v["stream"] = json!(stream_name(output.stream));
            v["text"] = json!(output.text);
            v
        }
        ProtoEvent::HookFinished(hook) => {
            let mut v = base("hook_finished");
            v["hookName"] = json!(hook.hook_name);
            v["exitCode"] = json!(hook.exit_code);
            v["durationMs"] = json!(hook.duration_ms);
            v
        }
        ProtoEvent::ActionFinished(finished) => {
            let mut v = base("action_finished");
            v["result"] = result_value(&context.action, finished.result);
            v
        }
        ProtoEvent::ActionFailed(failed) => {
            let mut v = base("action_failed");
            v["phase"] = json!(failed.phase.and_then(phase_name));
            v["message"] = json!(failed.message);
            v
        }
    };
    Some(value)
}

fn phase_name(phase: i32) -> Option<&'static str> {
    match phase {
        1 => Some("branch"),
        2 => Some("commit"),
        3 => Some("push"),
        4 => Some("pr"),
        _ => None,
    }
}

fn stream_name(stream: i32) -> &'static str {
    match stream {
        2 => "stderr",
        _ => "stdout",
    }
}

/// The contract result object, with the toast derived from the per-step
/// outcomes.
fn result_value(action: &str, result: Option<GitActionResult>) -> Value {
    let result = result.unwrap_or_default();
    let branch = branch_value(result.branch.unwrap_or_default());
    let commit = commit_value(result.commit.unwrap_or_default());
    let push = push_value(result.push.unwrap_or_default());
    let pr = pr_value(result.pr.unwrap_or_default());
    let toast = toast_value(&commit, &push, &pr);
    json!({
        "action": action,
        "branch": branch,
        "commit": commit,
        "push": push,
        "pr": pr,
        "toast": toast,
    })
}

fn branch_value(step: GitBranchStep) -> Value {
    let mut v = json!({ "status": step.status });
    if let Some(name) = non_empty(step.name) {
        v["name"] = json!(name);
    }
    v
}

fn commit_value(step: GitCommitStep) -> Value {
    let mut v = json!({ "status": step.status });
    if let Some(sha) = non_empty(step.commit_sha) {
        v["commitSha"] = json!(sha);
    }
    if let Some(subject) = non_empty(step.subject) {
        v["subject"] = json!(subject);
    }
    v
}

fn push_value(step: GitPushStep) -> Value {
    let mut v = json!({ "status": step.status });
    if let Some(branch) = non_empty(step.branch) {
        v["branch"] = json!(branch);
    }
    if let Some(upstream) = non_empty(step.upstream_branch) {
        v["upstreamBranch"] = json!(upstream);
    }
    if let Some(set_upstream) = step.set_upstream {
        v["setUpstream"] = json!(set_upstream);
    }
    v
}

fn pr_value(step: GitPrStep) -> Value {
    let mut v = json!({ "status": step.status });
    if let Some(url) = non_empty(step.url) {
        v["url"] = json!(url);
    }
    if let Some(number) = step.number {
        v["number"] = json!(number);
    }
    if let Some(base) = non_empty(step.base_branch) {
        v["baseBranch"] = json!(base);
    }
    if let Some(head) = non_empty(step.head_branch) {
        v["headBranch"] = json!(head);
    }
    if let Some(title) = non_empty(step.title) {
        v["title"] = json!(title);
    }
    v
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

/// The toast the toolbar shows once the action lands — the most consequential
/// step that ran, with a call to action when there is a pull request to open.
fn toast_value(commit: &Value, push: &Value, pr: &Value) -> Value {
    fn status(step: &Value) -> &str {
        step.get("status").and_then(Value::as_str).unwrap_or("")
    }
    let pr_status = status(pr);
    if pr_status == "created" || pr_status == "opened_existing" {
        if let Some(url) = pr.get("url").and_then(Value::as_str) {
            let title = if pr_status == "opened_existing" {
                "Pull request already open"
            } else {
                "Pull request opened"
            };
            return json!({
                "title": title,
                "cta": { "kind": "open_pr", "label": "View pull request", "url": url },
            });
        }
    }
    let title = if status(push) == "pushed" {
        "Pushed"
    } else if status(commit) == "created" {
        "Committed"
    } else {
        "Nothing to do"
    };
    json!({ "title": title, "cta": { "kind": "none" } })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sandbox_proto::GitActionEvent;
    use sandbox_proto::git_action_event::Event;

    fn context() -> ActionContext {
        ActionContext {
            action_id: "a-1".into(),
            cwd: "/workspace/task-7".into(),
            action: "commit_push_pr".into(),
        }
    }

    /// A conversation with a running Sandbox; only its id and Sandbox intent are
    /// read on the way to the control gate.
    fn running(task_id: &str) -> crate::dao::Conversation {
        crate::dao::Conversation {
            task_id: task_id.to_string(),
            workspace_id: Some("ws".into()),
            name: None,
            branch: None,
            agent_type: moatless_api_schemas::shared::AgentType(
                moatless_api_schemas::shared::AgentType::CLAUDE_CODE.to_string(),
            ),
            created_at: "2026-01-01T00:00:00Z".into(),
            created_at_db: None,
            updated_at: None,
            closed_at: None,
            turn_number: None,
            turn_status: None,
            turn_started_at: None,
            turn_ended_at: None,
            sandbox_desired_state: Some(moatless_api_schemas::enums::SandboxDesiredState::Running),
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

    /// A connector that fails the test if the action ever reaches the Sandbox —
    /// the control gate must refuse before any client is opened.
    struct Unreachable;

    #[async_trait::async_trait]
    impl moatless_backend_core::sandbox::SandboxConnector for Unreachable {
        async fn sandbox_client(
            &self,
            _task_id: &str,
        ) -> Result<
            moatless_backend_core::sandbox::SandboxClient,
            moatless_backend_core::sandbox::SandboxError,
        > {
            panic!("the Sandbox must not be reached when control is refused");
        }
    }

    /// A follower who may read a Task cannot commit, push or open a PR against
    /// it: `runStackedAction` gates on control, and a reader is answered as if
    /// the workspace were not there rather than told it exists.
    #[tokio::test]
    async fn a_reader_who_does_not_control_the_task_cannot_run_an_action() {
        let mut conversations = crate::dao::MockConversationDao::new();
        conversations
            .expect_list_followed_by_ids()
            .returning(|_, ids| Ok(vec![running(&ids[0])]));
        conversations
            .expect_controls_conversation()
            .returning(|_, _| Ok(false));
        let viewer = ScopeContext {
            user_id: "reader".into(),
            is_admin: false,
        };
        let payload = json!({
            "cwd": "/moatless/tasks/t-1",
            "actionId": "a-1",
            "action": "commit_push_pr",
        });

        let error = open_stacked_action(&conversations, &Unreachable, &viewer, &payload)
            .await
            .err()
            .expect("control is refused");
        assert!(matches!(
            error,
            OpenError::Access(WorkspaceAccessError::Unaddressable)
        ));
    }

    #[test]
    fn action_started_carries_the_phase_names_and_the_base_identifiers() {
        let event = GitActionEvent {
            event: Some(Event::ActionStarted(sandbox_proto::GitActionStarted {
                phases: vec![2, 3, 4],
            })),
        };
        let value = action_event(&context(), event).unwrap();
        assert_eq!(value["kind"], "action_started");
        assert_eq!(value["actionId"], "a-1");
        assert_eq!(value["cwd"], "/workspace/task-7");
        assert_eq!(value["action"], "commit_push_pr");
        assert_eq!(value["phases"], json!(["commit", "push", "pr"]));
    }

    #[test]
    fn a_hook_output_line_keeps_a_null_hook_name() {
        let event = GitActionEvent {
            event: Some(Event::HookOutput(sandbox_proto::GitActionHookOutput {
                hook_name: None,
                stream: 2,
                text: "commitlint: subject may not be empty".into(),
            })),
        };
        let value = action_event(&context(), event).unwrap();
        assert_eq!(value["kind"], "hook_output");
        assert_eq!(value["hookName"], Value::Null);
        assert_eq!(value["stream"], "stderr");
        assert_eq!(value["text"], "commitlint: subject may not be empty");
    }

    #[test]
    fn action_finished_builds_the_result_and_a_pull_request_toast() {
        let event = GitActionEvent {
            event: Some(Event::ActionFinished(sandbox_proto::GitActionFinished {
                result: Some(GitActionResult {
                    branch: Some(GitBranchStep {
                        status: "skipped_not_requested".into(),
                        name: None,
                    }),
                    commit: Some(GitCommitStep {
                        status: "created".into(),
                        commit_sha: Some("abc123".into()),
                        subject: Some("Fix the thing".into()),
                    }),
                    push: Some(GitPushStep {
                        status: "pushed".into(),
                        branch: Some("t3/fix".into()),
                        upstream_branch: Some("origin/t3/fix".into()),
                        set_upstream: Some(true),
                    }),
                    pr: Some(GitPrStep {
                        status: "created".into(),
                        url: Some("https://github.com/o/r/pull/9".into()),
                        number: Some(9),
                        base_branch: None,
                        head_branch: Some("t3/fix".into()),
                        title: Some("Fix the thing".into()),
                    }),
                }),
            })),
        };
        let value = action_event(&context(), event).unwrap();
        assert_eq!(value["kind"], "action_finished");
        let result = &value["result"];
        assert_eq!(result["action"], "commit_push_pr");
        assert_eq!(result["commit"]["commitSha"], "abc123");
        assert_eq!(result["push"]["setUpstream"], true);
        assert_eq!(result["pr"]["number"], 9);
        assert_eq!(result["toast"]["cta"]["kind"], "open_pr");
        assert_eq!(
            result["toast"]["cta"]["url"],
            "https://github.com/o/r/pull/9"
        );
    }

    #[test]
    fn action_failed_names_the_phase_and_message() {
        let event = GitActionEvent {
            event: Some(Event::ActionFailed(sandbox_proto::GitActionFailed {
                phase: Some(2),
                message: "commit-msg hook rejected the subject".into(),
            })),
        };
        let value = action_event(&context(), event).unwrap();
        assert_eq!(value["kind"], "action_failed");
        assert_eq!(value["phase"], "commit");
        assert_eq!(value["message"], "commit-msg hook rejected the subject");
    }

    #[test]
    fn an_unknown_action_string_is_rejected() {
        assert!(proto_action("commit_push_pr").is_some());
        assert!(proto_action("rebase").is_none());
    }

    #[test]
    fn discovery_reports_git_and_a_connected_github_account_authenticated() {
        let identity = crate::ports::GithubIdentity {
            account: Some("octocat".to_string()),
        };
        let value = discovery(Some(&identity));
        assert_eq!(value["versionControlSystems"][0]["kind"], "git");
        assert_eq!(value["versionControlSystems"][0]["status"], "available");
        assert_eq!(value["versionControlSystems"][0]["implemented"], true);
        // Effect `Schema.Option` wire form.
        assert_eq!(value["versionControlSystems"][0]["version"]["_tag"], "None");

        let provider = &value["sourceControlProviders"][0];
        assert_eq!(provider["kind"], "github");
        assert_eq!(provider["status"], "available");
        assert_eq!(provider["auth"]["status"], "authenticated");
        assert_eq!(provider["auth"]["account"]["_tag"], "Some");
        assert_eq!(provider["auth"]["account"]["value"], "octocat");
    }

    #[test]
    fn discovery_reports_a_connected_account_with_no_recorded_login_as_authenticated() {
        let identity = crate::ports::GithubIdentity { account: None };
        let auth = &discovery(Some(&identity))["sourceControlProviders"][0]["auth"];
        assert_eq!(auth["status"], "authenticated");
        assert_eq!(auth["account"]["_tag"], "None");
    }

    #[test]
    fn discovery_reports_a_viewer_with_no_github_credential_as_unauthenticated() {
        let auth = &discovery(None)["sourceControlProviders"][0]["auth"];
        assert_eq!(auth["status"], "unauthenticated");
        assert_eq!(auth["account"]["_tag"], "None");
    }

    fn started() -> GitActionEvent {
        GitActionEvent {
            event: Some(Event::ActionStarted(sandbox_proto::GitActionStarted {
                phases: vec![2, 3],
            })),
        }
    }

    fn finished() -> GitActionEvent {
        GitActionEvent {
            event: Some(Event::ActionFinished(sandbox_proto::GitActionFinished {
                result: Some(GitActionResult::default()),
            })),
        }
    }

    /// Drain the frames a relay produced from a fixed event sequence, returning
    /// the `kind` of each `Chunk`'s single value in order.
    async fn relayed_kinds(events: Vec<GitActionEvent>) -> Vec<String> {
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let stream = futures::stream::iter(events.into_iter().map(Ok));
        forward_action_stream(
            stream,
            context(),
            super::super::RequestId::for_test("r-1"),
            tx,
        )
        .await;
        let mut kinds = Vec::new();
        while let Ok(frame) = rx.try_recv() {
            assert_eq!(frame["_tag"], "Chunk");
            kinds.push(frame["values"][0]["kind"].as_str().unwrap().to_string());
        }
        kinds
    }

    #[tokio::test]
    async fn the_relay_forwards_every_event_in_order_and_terminates_on_finished() {
        let kinds = relayed_kinds(vec![
            started(),
            GitActionEvent {
                event: Some(Event::PhaseStarted(sandbox_proto::GitActionPhaseStarted {
                    phase: 2,
                    label: "Committing".into(),
                })),
            },
            finished(),
        ])
        .await;
        assert_eq!(
            kinds,
            ["action_started", "phase_started", "action_finished"]
        );
    }

    #[tokio::test]
    async fn the_relay_terminates_on_failed() {
        let kinds = relayed_kinds(vec![
            started(),
            GitActionEvent {
                event: Some(Event::ActionFailed(sandbox_proto::GitActionFailed {
                    phase: Some(2),
                    message: "hook rejected the subject".into(),
                })),
            },
        ])
        .await;
        assert_eq!(kinds, ["action_started", "action_failed"]);
    }

    #[tokio::test]
    async fn the_relay_returns_the_branch_the_action_created() {
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let finished = GitActionEvent {
            event: Some(Event::ActionFinished(sandbox_proto::GitActionFinished {
                result: Some(GitActionResult {
                    branch: Some(GitBranchStep {
                        status: "created".into(),
                        name: Some("t3/fix-the-thing".into()),
                    }),
                    ..Default::default()
                }),
            })),
        };
        let branch = forward_action_stream(
            futures::stream::iter(vec![Ok(started()), Ok(finished)]),
            context(),
            super::super::RequestId::for_test("r-3"),
            tx,
        )
        .await;
        assert_eq!(branch.as_deref(), Some("t3/fix-the-thing"));
    }

    #[tokio::test]
    async fn the_relay_returns_no_branch_when_the_checkout_did_not_move() {
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let branch = forward_action_stream(
            futures::stream::iter(vec![Ok(started()), Ok(finished())]),
            context(),
            super::super::RequestId::for_test("r-4"),
            tx,
        )
        .await;
        assert_eq!(branch, None);
    }

    #[tokio::test]
    async fn a_sandbox_stream_error_ends_the_relay_with_a_failure_frame() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let stream = futures::stream::iter(vec![
            Ok(started()),
            Err(tonic::Status::internal("sandbox went away")),
        ]);
        forward_action_stream(
            stream,
            context(),
            super::super::RequestId::for_test("r-2"),
            tx,
        )
        .await;
        let first = rx.try_recv().unwrap();
        assert_eq!(first["values"][0]["kind"], "action_started");
        // The error becomes an Exit/Failure frame, not another Chunk.
        let second = rx.try_recv().unwrap();
        assert_eq!(second["_tag"], "Exit");
    }
}
