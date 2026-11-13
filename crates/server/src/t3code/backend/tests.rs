//! Unit tests for the pure translations: a binding's pull request, a refusal's
//! shape, and which agent a thread created from T3 runs.

use super::commands::{create_refusal, default_agent_for, free_thread_id};
use super::pull_requests::pull_request_error;
use moatless_t3code::ports::{CommandRefusal, PullRequestError};
use moatless_tasks::agent::catalog;
use moatless_tasks::task::error::TaskError;
use moatless_tasks::task::models::{Task, TaskStatus};
use moatless_tasks::task::operations::TaskCommands;
use moatless_types::access_control::ScopeContext;
use std::sync::Arc;

/// The instance a person picks in T3 is what decides which agent runs the
/// thread. Getting this backwards creates a Claude Task for somebody who asked
/// for Codex, and the harness cannot be changed afterwards.
#[test]
fn each_instance_maps_to_the_first_catalog_agent_of_its_kind() {
    assert_eq!(
        default_agent_for("claudeAgent"),
        catalog::CLAUDE_CODE.profile
    );
    assert_eq!(default_agent_for("codex"), catalog::CODEX.profile);
}

/// An instance this server never published cannot arrive from a client that
/// read our provider list, so it is a bug rather than a choice. Claude is the
/// safe floor: it is the deployment's default harness.
#[test]
fn an_unknown_instance_falls_back_to_claude() {
    assert_eq!(
        default_agent_for("somethingElse"),
        moatless_types::agent::AgentProfile::ClaudeCode
    );
}

/// Pins the two `AdapterError` arms the attach-PR dialog matches by name.
///
/// A later rename can't silently reclassify them as this deployment's own bug;
/// the rest fall through a wildcard.
#[test]
fn the_two_recognised_adapter_errors_stay_actionable() {
    use moatless_adapter_model::error::{AdapterError, SharedAdapterError};

    let missing_credential = pull_request_error(AdapterError::upstream_credential(
        "resolve the GitHub token for user u-1",
        moatless_identity::auth::error::AuthError::Forbidden,
    ));
    assert!(matches!(
        missing_credential,
        PullRequestError::Unavailable(message)
            if message == "Connect a GitHub account to resolve pull requests."
    ));

    let provider_said_no =
        pull_request_error(SharedAdapterError::AdapterApi("PR is locked".into()).into());
    assert!(matches!(
        provider_said_no,
        PullRequestError::Unavailable(message) if message == "PR is locked"
    ));
}

/// A spent GitHub quota is the viewer's to wait out, not this deployment's
/// failure: as `Failed` every read until the reset logged at ERROR.
#[test]
fn a_spent_github_quota_says_when_to_try_again() {
    use moatless_adapter_model::error::SharedAdapterError;

    let limited = pull_request_error(
        SharedAdapterError::RateLimited {
            detail: "GitHub get_pr returned 403 Forbidden".into(),
            retry_at: chrono::DateTime::from_timestamp(1_793_002_200, 0),
        }
        .into(),
    );
    assert!(matches!(
        limited,
        PullRequestError::Unavailable(message)
            if message == "GitHub's API rate limit for this account is spent; try again after 08:10 UTC."
    ));

    let untimed = pull_request_error(
        SharedAdapterError::RateLimited {
            detail: "GitHub get_pr returned 429".into(),
            retry_at: None,
        }
        .into(),
    );
    assert!(matches!(
        untimed,
        PullRequestError::Unavailable(message)
            if message == "GitHub's API rate limit for this account is spent; try again shortly."
    ));
}

/// A conflict is the one refusal a person can clear themselves, so it is the
/// one whose wording has to survive the trip. `Rejected` rather than `Failed`
/// because nothing failed: the request was understood and answered.
#[test]
fn a_conflict_keeps_the_message_that_says_how_to_clear_it() {
    let refusal = create_refusal(
        "task-a",
        TaskError::Conflict(
            "task task-a has turn 3 in flight; fork at a turn before it, or wait for it to finish"
                .into(),
        ),
    );

    assert!(matches!(
        refusal,
        CommandRefusal::Rejected { message }
            if message
                == "task task-a has turn 3 in flight; fork at a turn before it, or wait for it to finish"
    ));
}

/// A preflight refusal names its own remedy, such as a GitHub rate limit to
/// wait out. As `NotWritable` it would read as a permission problem on the
/// Thread.
#[test]
fn a_preflight_refusal_keeps_its_remedy() {
    let message = "The github token is rate-limited on github.com, so its access to \
                   soaplabs/soap-platform-ci was not checked. Try again after 10:23 UTC.";
    let refusal = create_refusal("task-a", TaskError::Preflight(message.into()));

    assert!(matches!(
        refusal,
        CommandRefusal::Rejected { message: sent } if sent == message
    ));
}

/// The wildcard is what keeps a database or store failure from describing this
/// deployment's insides to a client, so pin it: a new `TaskError` variant is
/// only safe to name above once somebody has read its message.
#[test]
fn an_unclassified_failure_says_nothing_about_itself() {
    let refusal = create_refusal(
        "task-a",
        TaskError::Internal("connection to 10.0.3.7:5432 refused".into()),
    );

    assert!(matches!(
        refusal,
        CommandRefusal::Failed { message } if message == "The action could not be completed."
    ));
}

mod thread_ids {
    use super::*;
    use moatless_tasks::task::test_support::{ScriptedTasks, TaskStoreCalls};
    use moatless_types::access_control::Scope;

    const DRAFT: &str = "thread-minted-by-the-client";

    fn viewer() -> ScopeContext {
        ScopeContext::user("u-1")
    }

    fn thread_of(owner: &str, status: TaskStatus) -> Task {
        Task {
            task_id: DRAFT.to_string(),
            short_id: None,
            created_at: chrono::Utc::now(),
            last_message_at: chrono::Utc::now(),
            agent_profile: moatless_types::agent::AgentProfile::ClaudeCode,
            repository_id: Some("repo-1".to_string()),
            status,
            total_cost_usd: 0.0,
            site_url: None,
            template_id: None,
            vibe: None,
            name: None,
            description: None,
            site_id: None,
            site_provider: None,
            branch: None,
            model: None,
            created_by: Some(owner.to_string()),
            reporter_user_id: None,
            visibility: moatless_types::task::Visibility::Private,
            scope: Scope::User,
            owner_user_id: Some(owner.to_string()),
            run_as_user_id: None,
            updated_at: None,
            append_system_prompt: None,
            headless: false,
            ttl_minutes: 0,
            close_after_settled_days: 0,
            settle_after_hours: None,
            loop_id: None,
            workspace_id: None,
            sandbox_id: None,
            checkout_mode: None,
            memory_preload: Vec::new(),
            agent_properties: Default::default(),
            skills: Vec::new(),
        }
    }

    fn holding(
        existing: Option<Task>,
        frees_errored_ids: bool,
    ) -> (Arc<TaskCommands>, Arc<TaskStoreCalls>) {
        ScriptedTasks::new()
            .holding(existing)
            .frees_errored_ids(frees_errored_ids)
            .commands()
    }

    /// The ordinary create: nothing holds the id, so nothing is deleted on the
    /// way to it.
    #[tokio::test]
    async fn an_unused_id_is_taken_without_a_delete() {
        let (tasks, calls) = holding(None, false);

        free_thread_id(&tasks, &viewer(), DRAFT).await.unwrap();

        assert!(calls.delete_errored_empty_task().is_empty());
    }

    /// Regression: a preflight failure leaves the row `Error`, and the client
    /// retries with the same draft id.
    ///
    /// Refusing that retry would leave the draft dead forever, since nothing
    /// rotates the id.
    #[tokio::test]
    async fn a_failed_bootstrap_can_be_retried_under_the_same_id() {
        let (tasks, calls) = holding(Some(thread_of("u-1", TaskStatus::Error)), true);

        free_thread_id(&tasks, &viewer(), DRAFT).await.unwrap();

        let deletes = calls.delete_errored_empty_task();
        assert_eq!(deletes.len(), 1);
        assert_eq!(deletes[0].0, DRAFT);
        assert_eq!(deletes[0].1.user_id, "u-1");
    }

    /// A thread that exists and works is not something a create may take over.
    /// The store answers `false` for it — the row is not `Error`, or it has
    /// messages — and that is a refusal, not a second incarnation.
    #[tokio::test]
    async fn a_live_thread_still_refuses_a_second_create() {
        let (tasks, calls) = holding(Some(thread_of("u-1", TaskStatus::Open)), false);

        let refusal = free_thread_id(&tasks, &viewer(), DRAFT).await.unwrap_err();
        assert!(matches!(
            refusal,
            CommandRefusal::Rejected { message } if message == "That thread already exists."
        ));
        assert_eq!(calls.delete_errored_empty_task().len(), 1);
    }

    /// Somebody else's errored thread is readable when it is public, and it is
    /// still not this viewer's id to reuse. The refusal is the one that says
    /// nothing about what is there, and no delete is attempted.
    #[tokio::test]
    async fn another_users_errored_thread_is_not_reusable() {
        let (tasks, calls) = holding(Some(thread_of("u-2", TaskStatus::Error)), true);

        let refusal = free_thread_id(&tasks, &viewer(), DRAFT).await.unwrap_err();
        assert!(matches!(
            refusal,
            CommandRefusal::NotWritable { task_id } if task_id == DRAFT
        ));
        assert!(calls.delete_errored_empty_task().is_empty());
    }
}
