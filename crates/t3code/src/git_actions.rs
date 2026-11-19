//! The `git.*` write methods: `git.runStackedAction` and the pull-request
//! dialog.
//!
//! This module composes and sequences nothing: it turns a `cwd` into a Task,
//! refuses what the viewer may not reach, and hands the work to its agent.
//!
//! Pushing, committing and opening a PR happen through the agent, not here or
//! in the Sandbox — a toolbar press sends it the same kind of message any other
//! request would.

use moatless_types::access_control::ScopeContext;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::dao::ConversationDao;
use crate::nominal_path;
use crate::ports::{
    BoundPullRequestSource, PullRequestError, ResolvedPullRequest, ThreadCommandSink, ThreadTurn,
};
use crate::workspace;
use crate::workspace::WorkspaceAccessError;
use crate::workspace_listing::WorkspaceListing;

/// A stacked action that reached the agent, echoed back so the caller can stamp
/// the identifiers the client pressed onto the frames it answers with.
pub struct AskedAgent {
    pub action_id: String,
    pub cwd: String,
    pub action: String,
}

/// Why a stacked action could not be handed to the agent.
///
/// `Rejected` is a malformed request; `Access` is unreachable or not
/// writable; `Send` is a write that failed after being accepted.
#[derive(Debug)]
pub enum ActionError {
    Access(WorkspaceAccessError),
    Rejected(String),
    Send(crate::ports::CommandRefusal),
}

/// The contract's `GitRunStackedActionInput`.
///
/// The fields `run_stacked_action` refuses without are optional here, so that a
/// request missing one is answered with that refusal rather than a defect.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitRunStackedActionInput {
    cwd: Option<String>,
    action_id: Option<String>,
    action: Option<String>,
    commit_message: Option<String>,
    feature_branch: Option<bool>,
    file_paths: Option<Vec<String>>,
}

/// The contract's `GitPullRequestRefInput`; optional for the reason
/// [`GitRunStackedActionInput`]'s fields are.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitPullRequestRefInput {
    cwd: Option<String>,
    reference: Option<String>,
}

/// The contract's `GitPreparePullRequestThreadInput`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitPreparePullRequestThreadInput {
    #[serde(flatten)]
    reference: GitPullRequestRefInput,
    mode: Option<String>,
}

/// `value` trimmed, or `None` when that leaves nothing.
fn nonblank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

/// Resolve the `cwd`, refuse what the viewer cannot reach, and ask the Task's
/// agent to run the action.
///
/// Nothing touches the checkout here — commit, push and PR are the agent's
/// work; this only starts that turn.
pub async fn run_stacked_action(
    conversations: &dyn ConversationDao,
    sink: &dyn ThreadCommandSink,
    viewer: &ScopeContext,
    input: &GitRunStackedActionInput,
) -> Result<AskedAgent, ActionError> {
    let cwd = nonblank(input.cwd.as_deref())
        .ok_or_else(|| ActionError::Rejected("cwd is required".to_string()))?
        .to_string();
    let action_id = nonblank(input.action_id.as_deref())
        .ok_or_else(|| ActionError::Rejected("actionId is required".to_string()))?
        .to_string();
    let action = input
        .action
        .clone()
        .ok_or_else(|| ActionError::Rejected("action is required".to_string()))?;
    if !is_known_action(&action) {
        return Err(ActionError::Rejected(format!("unknown action `{action}`")));
    }

    let commit_message = nonblank(input.commit_message.as_deref()).map(str::to_string);
    let feature_branch = input.feature_branch.unwrap_or(false);
    let file_paths: Vec<String> = input
        .file_paths
        .iter()
        .flatten()
        .filter_map(|path| nonblank(Some(path.as_str())))
        .map(str::to_string)
        .collect();

    // The action commits, pushes and opens PRs, so reading is not enough: the
    // viewer must control the Task. Control and not write, because a controller
    // reaches the same three things from a shell with `git` and `gh`.
    let task_id = workspace::written_conversation(conversations, viewer, &cwd)
        .await
        .map_err(ActionError::Access)?;

    let text = compose_message(
        &action,
        commit_message.as_deref(),
        feature_branch,
        &file_paths,
    );
    sink.send_turn(
        viewer,
        ThreadTurn {
            task_id,
            text,
            // Server-minted: this turn originates from a button press, not from
            // the client's optimistic composer, so it has no client id to reuse.
            message_uuid: uuid::Uuid::new_v4().to_string(),
            model: None,
            harness: None,
            agent_mode: None,
            effort: None,
            attachments: Vec::new(),
            // A button press composes its own prompt, so there is no `$name`
            // mention in it to activate anything.
            skills: Vec::new(),
            queue: false,
        },
    )
    .await
    .map_err(ActionError::Send)?;

    Ok(AskedAgent {
        action_id,
        cwd,
        action,
    })
}

fn is_known_action(action: &str) -> bool {
    matches!(
        action,
        "commit" | "push" | "create_pr" | "commit_push" | "commit_push_pr"
    )
}

/// The message a button press becomes: a plain-English request for the agent to
/// run the git action, carrying whatever the toolbar collected — a hand-written
/// commit subject, a new-branch choice, a narrowed set of files.
fn compose_message(
    action: &str,
    commit_message: Option<&str>,
    feature_branch: bool,
    file_paths: &[String],
) -> String {
    let verb = match action {
        "commit" => "commit the current changes",
        "push" => "push the current branch to its remote",
        "create_pr" => "open a pull request for the current changes",
        "commit_push" => "commit the current changes and push them to the remote",
        // create_pr and commit_push_pr both open the PR; commit_push_pr is the
        // full stack.
        _ => "commit the current changes, push them, and open a pull request",
    };
    use std::fmt::Write as _;

    let mut message = format!("Please {verb}");
    if feature_branch {
        message.push_str(", creating a new feature branch for the commit");
    }
    if file_paths.is_empty() {
        message.push('.');
    } else {
        let _ = write!(
            message,
            ". Only stage these files: {}",
            file_paths.join(", ")
        );
    }
    if let Some(subject) = commit_message {
        let _ = write!(message, "\n\nUse this commit message:\n\n{subject}");
    }
    message
}

/// The single `action_finished` event the client is answered with.
///
/// Every step is `skipped_not_requested` since nothing ran on the checkout;
/// `actionId`/`cwd` echo the button press so the client pairs them.
pub fn asked_agent_event(sent: &AskedAgent) -> Value {
    json!({
        "actionId": sent.action_id,
        "cwd": sent.cwd,
        "action": sent.action,
        "kind": "action_finished",
        "result": {
            "action": sent.action,
            "branch": { "status": "skipped_not_requested" },
            "commit": { "status": "skipped_not_requested" },
            "push": { "status": "skipped_not_requested" },
            "pr": { "status": "skipped_not_requested" },
            "toast": {
                "title": asked_agent_toast_title(&sent.action),
                "description": "The agent will take it from here in this thread.",
                "cta": { "kind": "none" },
            },
        },
    })
}

fn asked_agent_toast_title(action: &str) -> &'static str {
    match action {
        "commit" => "Asked the agent to commit",
        "push" => "Asked the agent to push",
        "create_pr" => "Asked the agent to open a pull request",
        "commit_push" => "Asked the agent to commit and push",
        _ => "Asked the agent to commit, push and open a pull request",
    }
}

/// The outcome of preparing a PR-thread: either the local-mode result the
/// client creates the draft thread from, or a refusal of worktree mode by name.
pub enum PreparedThread {
    Local(Value),
    WorktreeUnsupported,
}

/// Resolve a pull-request reference for the attach-PR dialog, against the
/// primary repository of the Workspace the `cwd` names. Answers T3's
/// `GitResolvePullRequestResult` (`{pullRequest}`).
pub async fn resolve_pull_request(
    workspaces: &dyn WorkspaceListing,
    conversations: &dyn ConversationDao,
    pull_requests: &dyn BoundPullRequestSource,
    viewer: &ScopeContext,
    input: &GitPullRequestRefInput,
) -> Result<Value, PullRequestError> {
    let (cwd, reference) = pull_request_ref_input(input)?;
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

/// Prepare a PR-thread: local mode resolves the reference and answers the head
/// branch to create the draft thread on, no worktree.
///
/// Worktree mode is refused by name, since this surface materializes nothing on
/// disk; each mode is answered explicitly.
pub async fn prepare_pull_request_thread(
    workspaces: &dyn WorkspaceListing,
    conversations: &dyn ConversationDao,
    pull_requests: &dyn BoundPullRequestSource,
    viewer: &ScopeContext,
    input: &GitPreparePullRequestThreadInput,
) -> Result<PreparedThread, PullRequestError> {
    let mode = input.mode.as_deref().map_or_default(str::trim);
    match mode {
        "worktree" => Ok(PreparedThread::WorktreeUnsupported),
        "local" => {
            let (cwd, reference) = pull_request_ref_input(&input.reference)?;
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
fn pull_request_ref_input(
    input: &GitPullRequestRefInput,
) -> Result<(String, String), PullRequestError> {
    let cwd = nonblank(input.cwd.as_deref())
        .ok_or_else(|| PullRequestError::Unavailable("cwd is required".to_string()))?
        .to_string();
    let reference = nonblank(input.reference.as_deref())
        .ok_or_else(|| PullRequestError::Unavailable("reference is required".to_string()))?;
    Ok((cwd, normalize_reference(reference)))
}

/// `#123` names pull request 123; every other reference is passed through for
/// the backend to resolve as a bare number or an `owner/repo#N`.
fn normalize_reference(reference: &str) -> String {
    reference
        .strip_prefix('#')
        .filter(|rest| !rest.is_empty() && rest.chars().all(|ch| ch.is_ascii_digit()))
        .map_or_else(|| reference.to_string(), str::to_string)
}

/// Resolve the primary repository the `cwd` names and ask the port to resolve
/// the reference against it.
async fn resolve_reference(
    workspaces: &dyn WorkspaceListing,
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
    workspaces: &dyn WorkspaceListing,
    conversations: &dyn ConversationDao,
    viewer: &ScopeContext,
    cwd: &str,
) -> Result<String, PullRequestError> {
    let workspace_id = if let Some(id) = nominal_path::workspace_id_from_project_root(cwd) {
        id.to_string()
    } else if let Some(task_id) = nominal_path::task_id_from_cwd(cwd) {
        conversations
            .readable_by_id(viewer, task_id)
            .await
            .map_err(|error| PullRequestError::Failed(error.to_string()))?
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
/// `server.discoverSourceControl`.
///
/// GitHub auth reflects the viewer's provider-token row, not session auth;
/// `Option` fields use the Effect wire form `Schema.Option` decodes.
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
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A sink that records the turns sent to it, so a test can assert both that
    /// a turn was sent and what the agent was asked. Every other command is a
    /// test-only `unreachable!` — none of them are on the git action's path.
    #[derive(Default)]
    struct RecordingSink {
        turns: Mutex<Vec<ThreadTurn>>,
        /// Refuse every send, standing in for the write failing after the
        /// request was accepted.
        refuse: bool,
    }

    #[async_trait::async_trait]
    impl ThreadCommandSink for RecordingSink {
        async fn send_turn(
            &self,
            _viewer: &ScopeContext,
            turn: ThreadTurn,
        ) -> Result<(), crate::ports::CommandRefusal> {
            if self.refuse {
                return Err(crate::ports::CommandRefusal::Failed {
                    message: "sandbox is gone".to_string(),
                });
            }
            self.turns.lock().unwrap().push(turn);
            Ok(())
        }
        async fn create_thread(
            &self,
            _viewer: &ScopeContext,
            _thread: crate::ports::NewThread,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never creates a thread")
        }
        async fn revert(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
            _keep_through: i32,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never rewinds a thread")
        }
        async fn update_meta(
            &self,
            _viewer: &ScopeContext,
            _meta: crate::ports::ThreadMeta,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never updates meta")
        }
        async fn interrupt(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never interrupts a turn")
        }
        async fn stop(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never stops a thread")
        }
        async fn force_stop(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
            _turn_number: Option<i32>,
        ) -> Result<moatless_tasks::task::service::ForceStop, crate::ports::CommandRefusal>
        {
            unreachable!("a git action never force stops a thread")
        }
        async fn set_idle_timeout(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
            _minutes: i32,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never changes an idle timeout")
        }
        async fn set_visibility(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
            _visibility: moatless_types::task::Visibility,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never changes visibility")
        }
        async fn archive(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never archives a thread")
        }
        async fn unarchive(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never unarchives a thread")
        }
        async fn follow(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never follows a thread")
        }
        async fn unfollow(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never unfollows a thread")
        }
        async fn settle(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never settles a thread")
        }
        async fn unsettle(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never unsettles a thread")
        }
        async fn steer_queued(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
            _message_uuid: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never steers a queued turn")
        }
        async fn snooze(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
            _snoozed_until: chrono::DateTime<chrono::Utc>,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never snoozes a thread")
        }
        async fn unsnooze(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never unsnoozes a thread")
        }
        async fn answer_question(
            &self,
            _viewer: &ScopeContext,
            _answer: crate::ports::QuestionAnswer,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never answers a question")
        }
        async fn fork(
            &self,
            _viewer: &ScopeContext,
            _fork: crate::ports::ForkThread,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never forks a thread")
        }
        async fn link_pull_request(
            &self,
            _viewer: &ScopeContext,
            _reference: crate::ports::ThreadPullRequestRef,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never links a pull request")
        }
        async fn unlink_pull_request(
            &self,
            _viewer: &ScopeContext,
            _reference: crate::ports::ThreadPullRequestRef,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never unlinks a pull request")
        }
        async fn link_issue(
            &self,
            _viewer: &ScopeContext,
            _reference: crate::ports::ThreadIssueRef,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never links an issue")
        }
        async fn unlink_issue(
            &self,
            _viewer: &ScopeContext,
            _reference: crate::ports::ThreadIssueRef,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!("a git action never unlinks an issue")
        }
        async fn change_queue(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
            _change: crate::ports::QueueChange,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn visit(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
            _visited_at: chrono::DateTime<chrono::Utc>,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn mark_unread(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn delete(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
            _only_if_unused: bool,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn pin(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
            _order_key: Option<String>,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn unpin(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn set_auto_settle(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
            _enabled: bool,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn set_pin_order_key(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
            _order_key: String,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn set_active_order_key(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
            _order_key: String,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn watch_pull_request(
            &self,
            _viewer: &ScopeContext,
            _reference: crate::ports::ThreadPullRequestRef,
            _watching: bool,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
    }

    fn viewer(user_id: &str) -> ScopeContext {
        ScopeContext::user(user_id)
    }

    fn action_payload(action: &str) -> GitRunStackedActionInput {
        serde_json::from_value(json!({
            "cwd": "/moatless/tasks/t-1",
            "actionId": "a-1",
            "action": action,
        }))
        .expect("a GitRunStackedActionInput")
    }

    /// Someone who may read a Task cannot commit, push or open a PR against it:
    /// the action gates on write. The sink is never reached.
    #[tokio::test]
    async fn a_reader_who_cannot_write_the_task_cannot_run_an_action() {
        let mut conversations = crate::dao::MockConversationDao::new();
        conversations
            .expect_readable_by_id()
            .returning(|_, id| Ok(Some(crate::dao::running_conversation(id))));
        conversations
            .expect_writes_conversation()
            .returning(|_, _| Ok(false));
        let sink = RecordingSink::default();

        let error = run_stacked_action(
            &conversations,
            &sink,
            &viewer("reader"),
            &action_payload("commit_push_pr"),
        )
        .await
        .err()
        .expect("write is refused");

        assert!(matches!(
            error,
            ActionError::Access(WorkspaceAccessError::NoWrite)
        ));
        assert!(sink.turns.lock().unwrap().is_empty());
    }

    /// A writer pressing a git action reaches the agent as one user turn on
    /// the addressed Task, and the client is answered that it was sent.
    #[tokio::test]
    async fn a_writer_sends_the_agent_a_turn_and_is_answered() {
        let mut conversations = crate::dao::MockConversationDao::new();
        conversations
            .expect_readable_by_id()
            .returning(|_, id| Ok(Some(crate::dao::running_conversation(id))));
        conversations
            .expect_writes_conversation()
            .returning(|_, _| Ok(true));
        let sink = RecordingSink::default();

        let sent = run_stacked_action(
            &conversations,
            &sink,
            &viewer("owner"),
            &action_payload("commit_push_pr"),
        )
        .await
        .expect("the turn is sent");

        let turns = sink.turns.lock().unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].task_id, "t-1");
        assert!(turns[0].text.contains("commit"));
        assert!(turns[0].text.contains("pull request"));
        assert!(!turns[0].message_uuid.is_empty());

        // The client pairs the answer with the button by the id and cwd it sent.
        assert_eq!(sent.action_id, "a-1");
        assert_eq!(sent.cwd, "/moatless/tasks/t-1");
        let event = asked_agent_event(&sent);
        assert_eq!(event["kind"], "action_finished");
        assert_eq!(event["actionId"], "a-1");
        assert_eq!(event["result"]["commit"]["status"], "skipped_not_requested");
        assert_eq!(event["result"]["toast"]["cta"]["kind"], "none");
        assert_eq!(
            event["result"]["toast"]["title"],
            "Asked the agent to commit, push and open a pull request"
        );
    }

    /// A send that fails after the request was accepted is reported as a send
    /// failure, not a rejection or an access refusal.
    #[tokio::test]
    async fn a_failed_send_is_reported_as_a_send_error() {
        let mut conversations = crate::dao::MockConversationDao::new();
        conversations
            .expect_readable_by_id()
            .returning(|_, id| Ok(Some(crate::dao::running_conversation(id))));
        conversations
            .expect_writes_conversation()
            .returning(|_, _| Ok(true));

        let error = run_stacked_action(
            &conversations,
            &RecordingSink {
                refuse: true,
                ..RecordingSink::default()
            },
            &viewer("owner"),
            &action_payload("commit"),
        )
        .await
        .err()
        .expect("the send fails");

        assert!(matches!(error, ActionError::Send(_)));
    }

    #[test]
    fn an_unknown_action_string_is_rejected_before_anything_is_read() {
        assert!(is_known_action("commit_push_pr"));
        assert!(!is_known_action("rebase"));
    }

    #[test]
    fn each_action_composes_a_message_that_names_the_git_work() {
        assert_eq!(
            compose_message("commit", None, false, &[]),
            "Please commit the current changes."
        );
        assert_eq!(
            compose_message("push", None, false, &[]),
            "Please push the current branch to its remote."
        );
        assert_eq!(
            compose_message("create_pr", None, false, &[]),
            "Please open a pull request for the current changes."
        );
        assert_eq!(
            compose_message("commit_push", None, false, &[]),
            "Please commit the current changes and push them to the remote."
        );
        assert_eq!(
            compose_message("commit_push_pr", None, false, &[]),
            "Please commit the current changes, push them, and open a pull request."
        );
    }

    #[test]
    fn a_composed_message_carries_the_branch_choice_files_and_commit_subject() {
        let message = compose_message(
            "commit",
            Some("Fix the flaky test"),
            true,
            &["src/a.rs".to_string(), "src/b.rs".to_string()],
        );
        assert!(message.contains("creating a new feature branch"));
        assert!(message.contains("Only stage these files: src/a.rs, src/b.rs"));
        assert!(message.ends_with("Use this commit message:\n\nFix the flaky test"));
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
}
