//! The `git.*` write methods: `git.runStackedAction` and the pull-request
//! dialog. This module composes nothing and sequences nothing — it turns one
//! `cwd` into one Task, refuses what the viewer may not reach, and, for a
//! stacked action, hands the work to the Task's own agent.
//!
//! Committing, pushing and opening a pull request are not done here or in the
//! Sandbox: pressing one of the toolbar's git actions sends the agent a message
//! asking it to do the work, so it runs the same review, hooks and push the
//! agent would run for any other request. This surface only turns the button
//! press into that message and answers the client that it was sent.

use moatless_types::access_control::ScopeContext;
use serde_json::{Value, json};

use crate::dao::ConversationDao;
use crate::workspace_dao::T3WorkspaceDao;
use crate::ports::{
    BoundPullRequestSource, PullRequestError, ResolvedPullRequest, ThreadCommandSink, ThreadTurn,
};
use crate::projection;
use crate::workspace::{self, WorkspaceAccessError};

/// A stacked action that reached the agent, echoed back so the caller can stamp
/// the identifiers the client pressed onto the frames it answers with.
pub struct AskedAgent {
    pub action_id: String,
    pub cwd: String,
    pub action: String,
}

/// Why a stacked action could not be handed to the agent. A malformed request is
/// the viewer's fault (`Rejected`); a Task that is unreachable or not the
/// viewer's to operate is a refusal the read path also gives (`Access`); a
/// message that could not be recorded is the write failing after it was
/// accepted (`Send`).
#[derive(Debug)]
pub enum ActionError {
    Access(WorkspaceAccessError),
    Rejected(String),
    Send(crate::ports::CommandRefusal),
}

/// Resolve the `cwd`, refuse what the viewer cannot reach, and send the Task's
/// agent a message asking it to run the action. Nothing touches the checkout
/// here: the commit, push and pull request are the agent's work, and this only
/// starts the turn that asks for it.
pub async fn run_stacked_action(
    conversations: &dyn ConversationDao,
    sink: &dyn ThreadCommandSink,
    viewer: &ScopeContext,
    payload: &Value,
) -> Result<AskedAgent, ActionError> {
    let cwd = payload
        .get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
        .ok_or_else(|| ActionError::Rejected("cwd is required".to_string()))?
        .to_string();
    let action_id = payload
        .get("actionId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| ActionError::Rejected("actionId is required".to_string()))?
        .to_string();
    let action = payload
        .get("action")
        .and_then(Value::as_str)
        .ok_or_else(|| ActionError::Rejected("action is required".to_string()))?
        .to_string();
    if !is_known_action(&action) {
        return Err(ActionError::Rejected(format!("unknown action `{action}`")));
    }

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
    let file_paths: Vec<String> = payload
        .get("filePaths")
        .and_then(Value::as_array)
        .map(|paths| {
            paths
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    // The action commits, pushes and opens PRs, so reading is not enough: the
    // viewer must control the Task. Control and not write, because a controller
    // reaches the same three things from a shell with `git` and `gh`.
    let task_id = workspace::controlled_conversation(conversations, viewer, &cwd)
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
            agent_mode: None,
            effort: None,
            attachments: Vec::new(),
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

/// The single `action_finished` event the client is answered with. The action
/// did not run on the checkout, so every step is `skipped_not_requested`; the
/// toast tells the person the agent was asked instead. The `actionId` and `cwd`
/// are the ones the button carried, so the client pairs the answer with the
/// press.
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
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A conversation with a running Sandbox; only its id and Sandbox intent are
    /// read on the way to the control gate.
    fn running(task_id: &str) -> crate::dao::Conversation {
        crate::dao::Conversation {
            task_id: task_id.to_string(),
            workspace_id: Some("ws".into()),
            name: None,
            branch: None,
            agent_type: moatless_types::agent::AgentType(
                moatless_types::agent::AgentType::CLAUDE_CODE.to_string(),
            ),
            created_at: "2026-01-01T00:00:00Z".into(),
            created_at_db: None,
            updated_at: None,
            closed_at: None,
            turn_number: None,
            turn_status: None,
            turn_started_at: None,
            turn_ended_at: None,
            sandbox_desired_state_raw: Some(
                moatless_types::sandbox::SandboxDesiredState::Running
                    .as_str()
                    .to_string(),
            ),
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
        }
    }

    /// A sink that records the turns sent to it, so a test can assert both that a
    /// turn was sent and what the agent was asked. Every other command is a
    /// test-only `unreachable!` — none of them are on the git action's path.
    #[derive(Default)]
    struct RecordingSink {
        turns: Mutex<Vec<ThreadTurn>>,
    }

    #[async_trait::async_trait]
    impl ThreadCommandSink for RecordingSink {
        async fn send_turn(
            &self,
            _viewer: &ScopeContext,
            turn: ThreadTurn,
        ) -> Result<(), crate::ports::CommandRefusal> {
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
    }

    /// A sink that refuses every send, standing in for the write failing after
    /// the request was accepted.
    struct RefusingSink;

    #[async_trait::async_trait]
    impl ThreadCommandSink for RefusingSink {
        async fn send_turn(
            &self,
            _viewer: &ScopeContext,
            _turn: ThreadTurn,
        ) -> Result<(), crate::ports::CommandRefusal> {
            Err(crate::ports::CommandRefusal::Failed {
                message: "sandbox is gone".to_string(),
            })
        }
        async fn create_thread(
            &self,
            _viewer: &ScopeContext,
            _thread: crate::ports::NewThread,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn update_meta(
            &self,
            _viewer: &ScopeContext,
            _meta: crate::ports::ThreadMeta,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn interrupt(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn stop(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn archive(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn unarchive(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn settle(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn unsettle(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn snooze(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
            _snoozed_until: chrono::DateTime<chrono::Utc>,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn unsnooze(
            &self,
            _viewer: &ScopeContext,
            _task_id: &str,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn answer_question(
            &self,
            _viewer: &ScopeContext,
            _answer: crate::ports::QuestionAnswer,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
        async fn fork(
            &self,
            _viewer: &ScopeContext,
            _fork: crate::ports::ForkThread,
        ) -> Result<(), crate::ports::CommandRefusal> {
            unreachable!()
        }
    }

    fn viewer(user_id: &str) -> ScopeContext {
        ScopeContext {
            user_id: user_id.into(),
            is_admin: false,
        }
    }

    fn action_payload(action: &str) -> Value {
        json!({
            "cwd": "/moatless/tasks/t-1",
            "actionId": "a-1",
            "action": action,
        })
    }

    /// Someone who may read a Task cannot commit, push or open a PR against it:
    /// the action gates on control. The sink is never reached.
    #[tokio::test]
    async fn a_reader_who_does_not_control_the_task_cannot_run_an_action() {
        let mut conversations = crate::dao::MockConversationDao::new();
        conversations
            .expect_readable_by_id()
            .returning(|_, id| Ok(Some(running(id))));
        conversations
            .expect_controls_conversation()
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
        .expect("control is refused");

        assert!(matches!(
            error,
            ActionError::Access(WorkspaceAccessError::NoControl)
        ));
        assert!(sink.turns.lock().unwrap().is_empty());
    }

    /// A controller pressing a git action reaches the agent as one user turn on
    /// the addressed Task, and the client is answered that it was sent.
    #[tokio::test]
    async fn a_controller_sends_the_agent_a_turn_and_is_answered() {
        let mut conversations = crate::dao::MockConversationDao::new();
        conversations
            .expect_readable_by_id()
            .returning(|_, id| Ok(Some(running(id))));
        conversations
            .expect_controls_conversation()
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
            .returning(|_, id| Ok(Some(running(id))));
        conversations
            .expect_controls_conversation()
            .returning(|_, _| Ok(true));

        let error = run_stacked_action(
            &conversations,
            &RefusingSink,
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
