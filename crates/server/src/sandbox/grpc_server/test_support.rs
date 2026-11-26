//! Fixtures the test modules in this directory build a service with.

use std::sync::Arc;

use moatless_core::config::Config;
use moatless_core::sandbox_jwt as jwt;
use moatless_identity::auth::github::registry::GitHubAppRegistry;
use moatless_ports::identity::CommitIdentity;
use moatless_tasks::task::operations::TaskCommands;

use super::authorize::SandboxAuthorizer;
use super::state::BackendSandboxServiceImpl;

use moatless_ports::identity::MockCommitIdentity;
use moatless_tasks::task::models::Task;
use moatless_tasks::task::test_support::ScriptedTasks;

/// Deterministic uuid for a readable test name: distinct names map to distinct
/// ids, equal names to equal ids.
pub(super) fn uid(name: &str) -> uuid::Uuid {
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, name.as_bytes())
}

/// A `Task` fixture with the given owner; all other columns are inert
/// defaults irrelevant to the access check.
pub(super) fn task_fixture(owner_user_id: Option<&str>) -> Task {
    Task {
        task_id: "task-1".to_string(),
        short_id: None,
        created_at: chrono::Utc::now(),
        last_message_at: chrono::Utc::now(),
        agent_profile: moatless_types::agent::AgentProfile::ClaudeCode,
        repository_id: Some("repo-1".to_string()),
        status: moatless_types::task::TaskStatus::Open,
        total_cost_usd: 0.0,
        template_id: None,
        name: None,
        description: None,
        branch: None,
        model: None,
        created_by: owner_user_id.map(|u| uid(u).to_string()),
        reporter_user_id: None,
        visibility: moatless_types::task::Visibility::Private,
        owner_user_id: owner_user_id.map(|u| uid(u).to_string()),
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

pub(super) fn claims_for(user_id: &str) -> jwt::UserTokenClaims {
    jwt::UserTokenClaims {
        user_id: uid(user_id).to_string(),
        login: user_id.to_string(),
        task_id: None,
        sandbox_id: None,
        iat: 0,
        exp: u64::MAX,
        iss: jwt::ISSUER.to_string(),
        aud: None,
        generation: 0,
    }
}

/// Build a service whose authorizer has only the repositories the control gate
/// consults wired; everything else is `None`/default.
pub(super) fn service_with(
    tasks: Arc<TaskCommands>,
    users: MockCommitIdentity,
) -> BackendSandboxServiceImpl {
    let users: Arc<dyn CommitIdentity> = Arc::new(users);
    BackendSandboxServiceImpl {
        events: None,
        handlers: None,
        ingress: None,
        task_token_secret: None,
        authorizer: SandboxAuthorizer::dev_unauthenticated(Some(tasks), Some(users)),
        github_apps: Arc::new(GitHubAppRegistry::default()),
        backend_config: Arc::new(Config::test_minimal()),
        s3: None,
        workspace_repository: None,
        s3_bucket: None,
        s3_key_prefix: String::new(),
        transcript_s3_prefix: String::new(),
        workspace_s3_prefix: String::new(),
        event_dedup: Arc::new(crate::sandbox::event_dedup::EventDedup::default()),
        admissions: tokio_util::task::TaskTracker::new(),
    }
}

/// A task handle whose scoped `find_by_id` returns `task` for any caller — i.e.
/// "the scope filter let the read through". Pass `None` to model a task that is
/// missing or invisible to the caller.
pub(super) fn tasks_returning(task: Option<Task>) -> Arc<TaskCommands> {
    ScriptedTasks::new().holding(task).commands().0
}

pub(super) fn users_regular() -> MockCommitIdentity {
    let mut m = MockCommitIdentity::new();
    m.expect_is_platform_admin().returning(|_| Ok(false));
    m
}

pub(super) fn users_admin() -> MockCommitIdentity {
    let mut m = MockCommitIdentity::new();
    m.expect_is_platform_admin().returning(|_| Ok(true));
    m
}
