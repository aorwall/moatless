#![cfg(feature = "db-tests")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    clippy::print_stderr
)]
//! What a viewer sees in the T3 compatibility module's shell listing.
//!
//! The listing's two halves are filtered by deliberately different rules, and
//! the difference is invisible without a real database: the access filter, the
//! follow filter and the widening are all SQL. These tests seed workspaces and
//! conversations across ownership, visibility, scope and following, and assert
//! per viewer.

use crate::common;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use chrono::{DateTime, Duration as ChronoDuration, SubsecRound, Utc};
use futures::future::BoxFuture;
use moatless_backend::t3code::backend as t3code_backend;
use moatless_core::change_feed::conversation::{ConversationChange, ConversationHead};
use moatless_core::change_feed::listing::record_change;
use moatless_identity::auth::api_keys::PgApiKeyDao;
use moatless_identity::auth::pairing::DevicePairing;
use moatless_identity::auth::pairing::PgPairingApiKeyIssuer;
use moatless_identity::auth::pairing::PgPairingCodeDao;
use moatless_ports::sandbox::{
    ClusterDiagnostics, ContractSandboxStatus, LiveStatus, LogOptions, SandboxDiagnosticsEntry,
    SandboxError, SandboxPortPublisher, ServerLogStream,
};
use moatless_t3code::dao::{
    Conversation, ConversationDao, ConversationMatch, InteractiveCall, PgConversationDao,
    Transcript,
};
use moatless_t3code::listing::{
    RESUME_WINDOW, UiRpcEnvironmentIdentity, UiRpcListingChange, UiRpcListingHead, UiRpcResume,
    current_head, resume,
};
use moatless_t3code::preview_tabs::InMemoryPreviewTabDao;
use moatless_t3code::{
    AgentModelDefaults, Deployment, ShellFeed, Sources, UiRpcDependencies, archived_shell_snapshot,
    files, router_from_env, shell_snapshot, shell_subscription, thread_shell, thread_snapshot,
};
use moatless_t3code::{
    SandboxConnector, SandboxIntentReader, SandboxObserver, SandboxStarter, SandboxStopper,
};
use moatless_t3code::{UiRpcAuthIdentity, UserRole};
use moatless_tasks::message::dao::MessageDao;
use moatless_tasks::message::dao::PgMessageDao;
use moatless_tasks::message::models::MessageType;
use moatless_tasks::message::models::TaskMessage;
use moatless_tasks::message::service as message_service;
use moatless_tasks::tag::dao::PgTagDao;
use moatless_tasks::tag::dao::TagDao;
use moatless_tasks::task::dao::TaskDao;
use moatless_tasks::task::dao::TaskPatch;
use moatless_tasks::task::models::SandboxDesiredState;
use moatless_tasks::task::models::SandboxStopReason;
use moatless_tasks::task::models::Task;
use moatless_tasks::task::models::TaskStatus;
use moatless_tasks::task::models::Visibility;
use moatless_tasks::task_activities::mapping::BlockContext;
use moatless_tasks::task_activities::mapping::ConversationWrite;
use moatless_tasks::task_activities::mapping::map_content;
use moatless_tasks::task_turn::dao::PgTaskTurnDao;
use moatless_tasks::task_turn::dao::TaskTurnDao;
use moatless_tasks::task_turn::dao::TurnStateEvent;
use moatless_tasks::task_turn::models::classify_turn_transition;
use moatless_types::access_control::{Scope, ScopeContext};
use moatless_types::agent::AgentType;
use moatless_workspaces::repository::dao as repository_dao;
use moatless_workspaces::repository::routes::repository_router;
use moatless_workspaces::workspace::dao::PgWorkspaceDao;
use moatless_workspaces::workspace::dao::WorkspaceDao;
use moatless_workspaces::workspace::dao::WorkspacePatch;
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use sqlx::{AssertSqlSafe, PgPool};
use tower::ServiceExt;

use common::{
    USER_1, authed_request, body_json, make_message_row, make_repository_row, make_task_row,
    make_workspace_row, run_config, seed_team, test_app_state,
};

/// Map one Message's content the way the write path does: text onto the row,
/// each call and each outcome onto the conversation. Tests go through this
/// same seam, rather than handing `append` a block list directly.
fn conversation_write(
    task_id: &str,
    uuid: &str,
    message_type: MessageType,
    content: &Value,
) -> ConversationWrite {
    map_content(
        BlockContext {
            task_id,
            message_uuid: uuid,
            parent_uuid: None,
            message_type,
            block_index: 0,
        },
        content,
    )
}

/// [`authed_request`] plus the identity the T3 handlers read.
///
/// The auth middleware inserts two extensions, not one: `AuthIdentity` for the
/// Moatless handlers and `UiRpcAuthIdentity` for the T3 ones. This request
/// bypasses that middleware, so a T3 route reached with only the first is a
/// `500` on a missing extension rather than the response under test.
fn ui_rpc_authed_request(
    method: &str,
    uri: &str,
    user_id: &str,
) -> axum::http::Request<axum::body::Body> {
    let mut req = authed_request(method, uri, user_id);
    req.extensions_mut().insert(UiRpcAuthIdentity {
        user_id: user_id.parse().expect("USER_1 is a UUID"),
        login: user_id.to_string(),
        role: UserRole::User,
        provider: "test".to_string(),
        group_id: None,
    });
    req
}

fn viewer(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.into(),
        is_admin: false,
    }
}

fn admin(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.into(),
        is_admin: true,
    }
}

fn defaults() -> AgentModelDefaults {
    AgentModelDefaults {
        claude_code: Some("claude-opus-4-5".into()),
        codex: Some("gpt-5.6-sol".into()),
    }
}

/// A deployment with no Sandboxes running.
///
/// The routes this suite drives — the shell listing and the public descriptor —
/// never reach a Sandbox. What one answers is a gRPC round trip against a pod
/// or the K8s API, and no test here has either.
struct NoSandboxes;

#[async_trait::async_trait]
impl SandboxConnector for NoSandboxes {
    async fn sandbox_client(
        &self,
        task_id: &str,
    ) -> Result<sandbox_proto::client::SandboxClient, SandboxError> {
        Err(SandboxError::NotReady(task_id.to_string()))
    }
}

#[async_trait::async_trait]
impl ClusterDiagnostics for NoSandboxes {
    async fn list_sandbox_diagnostics(&self) -> Result<Vec<SandboxDiagnosticsEntry>, SandboxError> {
        Ok(Vec::new())
    }
}

#[async_trait::async_trait]
impl SandboxObserver for NoSandboxes {
    async fn sandbox_status(
        &self,
        task_id: &str,
        _task_status: TaskStatus,
        _desired_state: SandboxDesiredState,
    ) -> Result<ContractSandboxStatus, SandboxError> {
        Err(SandboxError::NotReady(task_id.to_string()))
    }

    async fn live_status(
        &self,
        task_id: &str,
        _task_status: TaskStatus,
        _desired_state: SandboxDesiredState,
    ) -> Result<LiveStatus, SandboxError> {
        Err(SandboxError::NotReady(task_id.to_string()))
    }

    async fn server_logs(
        &self,
        task_id: &str,
        _server_name: &str,
        _options: LogOptions,
    ) -> Result<ServerLogStream, SandboxError> {
        Err(SandboxError::NotReady(task_id.to_string()))
    }
}

#[async_trait::async_trait]
impl SandboxIntentReader for NoSandboxes {
    async fn desired_state(&self, _task_id: &str) -> Result<SandboxDesiredState, SandboxError> {
        Ok(SandboxDesiredState::Stopped)
    }
}

#[async_trait::async_trait]
impl SandboxStarter for NoSandboxes {
    async fn start(&self, task_id: &str) -> Result<(), SandboxError> {
        Err(SandboxError::NotReady(task_id.to_string()))
    }
}

#[async_trait::async_trait]
impl SandboxStopper for NoSandboxes {
    async fn stop(&self, task_id: &str, _reason: SandboxStopReason) -> Result<(), SandboxError> {
        Err(SandboxError::NotReady(task_id.to_string()))
    }
}

#[async_trait::async_trait]
impl SandboxPortPublisher for NoSandboxes {
    async fn publish_port(
        &self,
        task_id: &str,
        _name: &str,
        _port: u16,
    ) -> Result<Option<String>, SandboxError> {
        Err(SandboxError::NotReady(task_id.to_string()))
    }
}

/// The compatibility router's dependencies, as `main` assembles them. Pairing
/// itself is asserted in `crates/server/tests/device_pairing_test.rs`; here it is
/// wired only because the router carries it.
fn mounted_dependencies(pool: &PgPool) -> UiRpcDependencies {
    UiRpcDependencies {
        session_cookie_name: "moatless_session".into(),
        session_jwt_secret: Some("test-secret".into()),
        pairing: Arc::new(DevicePairing::new(
            Arc::new(PgPairingCodeDao(pool.clone())),
            Arc::new(PgPairingApiKeyIssuer(Arc::new(PgApiKeyDao(pool.clone())))),
        )),
        workspaces: Arc::new(moatless_backend::t3code::conversations_over(pool)),
        workspace_script_commands: Arc::new(t3code_backend::WorkspaceScripts(Arc::new(
            moatless_workspaces::workspace::dao::PgWorkspaceDao(pool.clone()),
        ))),
        conversations: Arc::new(moatless_backend::t3code::conversations_over(pool)),
        sandboxes: Arc::new(NoSandboxes),
        // This suite drives no snapshot reads; a stopped Task answers as before.
        snapshots: None,
        observer: Arc::new(NoSandboxes),
        diagnostics: Arc::new(NoSandboxes),
        // Nothing this suite drives publishes a Script's port.
        ports: Arc::new(NoSandboxes),
        tasks: Arc::new(moatless_backend::t3code::conversations_over(pool)),
        intent: Arc::new(NoSandboxes),
        // Nothing this suite drives starts or stops a Sandbox.
        starter: Arc::new(NoSandboxes),
        stopper: Arc::new(NoSandboxes),
        // Nothing this suite drives opens a preview tab, and the in-process
        // store is a production path rather than a double — see the epoch note
        // on `PreviewTabDao`.
        preview_tabs: Arc::new(InMemoryPreviewTabDao::new()),
        // The write ports, wired exactly as `main` wires them. This suite drives
        // reads only; what they do is asserted in
        // `crates/server/tests/ui_rpc_command_test.rs`.
        agents: Arc::new(t3code_backend::AppStateAgents(test_app_state(pool.clone()))),
        branches: Arc::new(t3code_backend::AppStateBranches(test_app_state(
            pool.clone(),
        ))),
        skills: Arc::new(t3code_backend::AppStateSkills(test_app_state(pool.clone()))),
        relations: Arc::new(t3code_backend::AppStateTaskRelations(test_app_state(
            pool.clone(),
        ))),
        pull_requests: Arc::new(t3code_backend::AppStateBoundPullRequests(test_app_state(
            pool.clone(),
        ))),
        source_control_auth: Arc::new(t3code_backend::AppStateSourceControlAuth(test_app_state(
            pool.clone(),
        ))),
        commands: Arc::new(t3code_backend::AppStateCommands(test_app_state(
            pool.clone(),
        ))),
        attachments: Arc::new(t3code_backend::AppStateAttachments(test_app_state(
            pool.clone(),
        ))),
        usage: Arc::new(moatless_t3code::PgUsageDao::new(pool.clone())),
        model_defaults: defaults(),
        frontend_url: "http://localhost:3000".into(),
        otlp_traces_enabled: false,
    }
}

/// Every assertion below goes through here, so a query that Postgres refuses
/// fails with the database's own error text rather than as an empty listing.
/// The SQL in `crates/t3code/src/dao.rs` is not compile-checked — `sqlx::query_as` without
/// the macro is a string until a connection sees it — so this is the only place
/// a broken join, decode or parameter can announce itself.
async fn snapshot(pool: &PgPool, viewer: &ScopeContext) -> Value {
    match shell_snapshot(
        &moatless_backend::t3code::conversations_over(pool),
        &moatless_backend::t3code::conversations_over(pool),
        &defaults(),
        viewer,
    )
    .await
    {
        Ok(snapshot) => snapshot.value,
        Err(error) => panic!(
            "the shell listing query failed for viewer {} (is_admin={}): {error}\n\n{error:?}",
            viewer.user_id, viewer.is_admin
        ),
    }
}

/// The project ids in a viewer's listing, sorted for comparison.
async fn project_ids(pool: &PgPool, viewer: &ScopeContext) -> Vec<String> {
    let mut ids: Vec<String> = snapshot(pool, viewer).await["projects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|project| project["id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

/// The archived half of a viewer's listing — the closed conversations they
/// follow.
async fn archived_snapshot(pool: &PgPool, viewer: &ScopeContext) -> Value {
    archived_shell_snapshot(
        &moatless_backend::t3code::conversations_over(pool),
        &moatless_backend::t3code::conversations_over(pool),
        &defaults(),
        viewer,
    )
    .await
    .expect("the archived listing query failed")
}

/// The conversation ids in a viewer's archive, sorted for comparison.
async fn archived_thread_ids(pool: &PgPool, viewer: &ScopeContext) -> Vec<String> {
    let mut ids: Vec<String> = archived_snapshot(pool, viewer).await["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|thread| thread["id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

/// The conversation ids in a viewer's listing, sorted for comparison.
async fn thread_ids(pool: &PgPool, viewer: &ScopeContext) -> Vec<String> {
    let mut ids: Vec<String> = snapshot(pool, viewer).await["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|thread| thread["id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

/// One conversation's listing row asked for by id, whichever listing would or
/// would not have carried it.
async fn thread_shell_row(pool: &PgPool, viewer: &ScopeContext, id: &str) -> Option<Value> {
    thread_shell(
        &moatless_backend::t3code::conversations_over(pool),
        &moatless_backend::t3code::conversations_over(pool),
        &defaults(),
        viewer,
        id,
    )
    .await
    .expect("the listing row query failed")
}

async fn thread(pool: &PgPool, viewer: &ScopeContext, id: &str) -> Value {
    snapshot(pool, viewer).await["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["id"] == id)
        .unwrap_or_else(|| panic!("{id} is absent from the listing"))
        .clone()
}

async fn project(pool: &PgPool, viewer: &ScopeContext, id: &str) -> Value {
    snapshot(pool, viewer).await["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|project| project["id"] == id)
        .unwrap_or_else(|| panic!("{id} is absent from the listing"))
        .clone()
}

// =========================================================================
// Conversation seeding
// =========================================================================

/// Insert a task in `workspace_id`, owned by `owner`.
///
/// `insert_task` auto-follows for the creator, exactly as every production
/// creation path does, so the seeded state is the one a real deployment
/// reaches. Tests that need a non-follower unfollow explicitly.
async fn seed_task(pool: &PgPool, task_id: &str, owner: &str, workspace_id: Option<&str>) -> Task {
    let mut task = make_task_row(task_id, owner);
    task.workspace_id = workspace_id.map(str::to_string);
    moatless_tasks::task::dao::store_over(pool)
        .insert_task(&task)
        .await
        .unwrap();
    task
}

async fn seed_public_task(
    pool: &PgPool,
    task_id: &str,
    owner: &str,
    workspace_id: Option<&str>,
) -> Task {
    let mut task = make_task_row(task_id, owner);
    task.workspace_id = workspace_id.map(str::to_string);
    task.visibility = Visibility::Public;
    moatless_tasks::task::dao::store_over(pool)
        .insert_task(&task)
        .await
        .unwrap();
    task
}

async fn follow(pool: &PgPool, user_id: &str, task_id: &str) {
    moatless_tasks::task::dao::store_over(pool)
        .follow_task(user_id, task_id)
        .await
        .unwrap();
}

async fn set_settled_and_snoozed(pool: &PgPool, user_id: &str, task_id: &str) {
    let tasks = moatless_tasks::task::dao::store_over(pool);
    tasks
        .settle_task(user_id, task_id, Utc::now())
        .await
        .unwrap();
    tasks
        .snooze_task(
            user_id,
            task_id,
            Utc::now() + ChronoDuration::hours(1),
            Utc::now(),
        )
        .await
        .unwrap();
}

async fn follower_state(
    pool: &PgPool,
    user_id: &str,
    task_id: &str,
) -> (
    Option<String>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
) {
    sqlx::query_as(
        r#"SELECT settled_override, settled_at, snoozed_until, snoozed_at
           FROM task_follows
           WHERE user_id = $1 AND task_id = $2"#,
    )
    .bind(user_id)
    .bind(task_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn unfollow(pool: &PgPool, user_id: &str, task_id: &str) {
    moatless_tasks::task::dao::store_over(pool)
        .unfollow_task(user_id, task_id)
        .await
        .unwrap();
}

async fn seed_turn(pool: &PgPool, task_id: &str, turn_number: i32, status: &str) {
    sqlx::query(
        r#"INSERT INTO task_turns (task_id, turn_number, status, started_at)
           VALUES ($1, $2, $3, NOW())"#,
    )
    .bind(task_id)
    .bind(turn_number)
    .bind(status)
    .execute(pool)
    .await
    .unwrap();
}

/// Seed the Sandbox an ordinary Task runs in, named after the Task, and bind
/// the Task to it.
async fn seed_sandbox(pool: &PgPool, task_id: &str, desired_state: &str, started: bool) {
    let workspace_id: String = sqlx::query_scalar(
        "SELECT COALESCE(workspace_id, 'ws_' || repository_id) FROM tasks WHERE task_id = $1",
    )
    .bind(task_id)
    .fetch_one(pool)
    .await
    .unwrap();

    // `sandboxes.workspace_id` is an FK, and these fixtures name Workspaces that
    // `seed_workspaces` does not create. An already-seeded id conflicts, which
    // is the no-op this wants.
    let workspace = make_workspace_row(&workspace_id, "user-1");
    let _ = PgWorkspaceDao(pool.clone())
        .insert_workspace(&workspace)
        .await;

    sqlx::query(
        r#"INSERT INTO sandboxes (sandbox_id, workspace_id, desired_state, started_at)
           VALUES ($1, $2, $3, CASE WHEN $4 THEN NOW() ELSE NULL END)
           ON CONFLICT (sandbox_id) DO UPDATE SET
             workspace_id = EXCLUDED.workspace_id,
             desired_state = EXCLUDED.desired_state,
             started_at = EXCLUDED.started_at"#,
    )
    .bind(task_id)
    .bind(&workspace_id)
    .bind(desired_state)
    .bind(started)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query("UPDATE tasks SET sandbox_id = $1 WHERE task_id = $1")
        .bind(task_id)
        .execute(pool)
        .await
        .unwrap();
}

/// A message from a person, which is what the activity timestamp counts.
async fn seed_user_message(pool: &PgPool, task_id: &str, uuid: &str) {
    let msg = make_message_row(task_id, uuid);
    PgMessageDao(pool.clone())
        .append(&msg, &[], &ConversationWrite::default())
        .await
        .unwrap();
}

/// A message the agent produced. Stored as `system` with the SDK's `user` role
/// for tool results, and as `assistant` for the agent's own replies — neither
/// is a person writing.
async fn seed_agent_message(
    pool: &PgPool,
    task_id: &str,
    uuid: &str,
    turn_number: i32,
    model: Option<&str>,
) {
    let msg = TaskMessage {
        message_type: MessageType::Assistant,
        user_type: "system".into(),
        turn_number: Some(turn_number),
        assistant_message_model: model.map(str::to_string),
        ..make_message_row(task_id, uuid)
    };
    PgMessageDao(pool.clone())
        .append(&msg, &[], &ConversationWrite::default())
        .await
        .unwrap();
}

fn parse(value: &Value) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value.as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc)
}

// =========================================================================
// The contract, transcribed
// =========================================================================
//
// `OrchestrationThreadShell` and the types it nests, copied out of
// `packages/contracts/src/orchestration.ts` on `moatless-adapter`. "Optional"
// means the contract can fill it — `Schema.optional` or a decoding default.
// Everything else has to be on the wire, `null` included, or the whole
// snapshot fails to decode and the sidebar goes blank.
//
// The unit tests assert this against hand-built rows; here it is asserted
// against rows a real database produced, where the nullable columns are
// genuinely null rather than null because a factory said so.

/// `OrchestrationThreadShell` (`orchestration.ts:401-427`).
const THREAD_REQUIRED: &[&str] = &[
    "id",
    "projectId",
    "title",
    "modelSelection",
    "runtimeMode",
    "branch",
    "worktreePath",
    "latestTurn",
    "createdAt",
    "updatedAt",
    "session",
    "latestUserMessageAt",
    "hasPendingApprovals",
    "hasPendingUserInput",
    "hasActionableProposedPlan",
];
const THREAD_OPTIONAL: &[&str] = &[
    "backgroundLiveness",
    "linkedPullRequest",
    "linkedPullRequests",
    "interactionMode",
    "archivedAt",
    "settledOverride",
    "settledAt",
    "snoozedUntil",
    "snoozedAt",
];
const THREAD_NON_EMPTY: &[&str] = &["id", "projectId", "title"];

/// `ModelSelection` (`:64-68`), after the wire transform.
const MODEL_SELECTION_REQUIRED: &[&str] = &["instanceId", "model"];
const MODEL_SELECTION_OPTIONAL: &[&str] = &["options"];

/// `OrchestrationLatestTurn` (`:334-342`).
const TURN_REQUIRED: &[&str] = &[
    "turnId",
    "state",
    "requestedAt",
    "startedAt",
    "completedAt",
    "assistantMessageId",
];
const TURN_OPTIONAL: &[&str] = &["sourceProposedPlan"];

/// `OrchestrationSession` (`:272-281`).
const SESSION_REQUIRED: &[&str] = &[
    "threadId",
    "status",
    "providerName",
    "activeTurnId",
    "lastError",
    "updatedAt",
];
const SESSION_OPTIONAL: &[&str] = &["providerInstanceId", "runtimeMode"];

fn assert_keys(value: &Value, what: &str, required: &[&str], optional: &[&str]) {
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("{what} must be an object"));
    for key in required {
        assert!(object.contains_key(*key), "{what} is missing `{key}`");
    }
    for key in object.keys() {
        assert!(
            required.contains(&key.as_str()) || optional.contains(&key.as_str()),
            "{what} emits `{key}`, which the contract does not declare"
        );
    }
}

fn assert_non_empty(value: &Value, what: &str, keys: &[&str]) {
    for key in keys {
        let text = value[*key]
            .as_str()
            .unwrap_or_else(|| panic!("{what}.{key} must be a string"));
        assert_eq!(text, text.trim(), "{what}.{key} must be trimmed");
        assert!(!text.is_empty(), "{what}.{key} must not be empty");
    }
}

/// Every key the client reads off one conversation row, asserted against the
/// contract. Nested objects are checked too — an undeclared key one level down
/// fails the decode just as surely as one at the top.
fn assert_decodable_thread(thread: &Value) {
    assert_keys(thread, "thread", THREAD_REQUIRED, THREAD_OPTIONAL);
    assert_non_empty(thread, "thread", THREAD_NON_EMPTY);
    assert_keys(
        &thread["modelSelection"],
        "modelSelection",
        MODEL_SELECTION_REQUIRED,
        MODEL_SELECTION_OPTIONAL,
    );
    assert_non_empty(
        &thread["modelSelection"],
        "modelSelection",
        MODEL_SELECTION_REQUIRED,
    );
    if !thread["latestTurn"].is_null() {
        assert_keys(
            &thread["latestTurn"],
            "latestTurn",
            TURN_REQUIRED,
            TURN_OPTIONAL,
        );
    }
    if !thread["session"].is_null() {
        assert_keys(
            &thread["session"],
            "session",
            SESSION_REQUIRED,
            SESSION_OPTIONAL,
        );
    }
}

/// Seeds one workspace per ownership/scope combination the listing has to
/// distinguish: the viewer's own, a global one they do not own, someone else's
/// private one, and a soft-deleted one of their own.
async fn seed_workspaces(pool: &PgPool) {
    let repo = PgWorkspaceDao(pool.clone());

    repo.insert_workspace(&make_workspace_row("ws-own", "user-1"))
        .await
        .unwrap();

    let mut global = make_workspace_row("ws-global", "user-2");
    global.scope = Scope::Global;
    global.owner_user_id = None;
    global.created_by = None;
    repo.insert_workspace(&global).await.unwrap();

    repo.insert_workspace(&make_workspace_row("ws-other", "user-2"))
        .await
        .unwrap();

    let mut deleted = make_workspace_row("ws-deleted", "user-1");
    deleted.deleted = true;
    repo.insert_workspace(&deleted).await.unwrap();
}

#[sqlx::test]
async fn a_viewer_sees_the_projects_they_may_read(pool: PgPool) {
    seed_workspaces(&pool).await;

    // Own and global — including the global one they do not own. Someone
    // else's private workspace and a deleted one are absent.
    assert_eq!(
        project_ids(&pool, &viewer("user-1")).await,
        vec!["ws-global".to_string(), "ws-own".to_string()]
    );
}

#[sqlx::test]
async fn an_administrator_sees_every_project_in_the_deployment(pool: PgPool) {
    seed_workspaces(&pool).await;

    assert_eq!(
        project_ids(&pool, &admin("root")).await,
        vec![
            "ws-global".to_string(),
            "ws-other".to_string(),
            "ws-own".to_string()
        ]
    );
}

#[sqlx::test]
async fn deleted_workspaces_are_absent_even_for_an_administrator(pool: PgPool) {
    seed_workspaces(&pool).await;

    assert!(
        !project_ids(&pool, &admin("root"))
            .await
            .contains(&"ws-deleted".to_string())
    );
}

#[sqlx::test]
async fn two_workspaces_are_two_groups_not_one(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());
    repo.insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    repo.insert_workspace(&make_workspace_row("ws-b", "user-1"))
        .await
        .unwrap();

    let viewer = viewer("user-1");
    let a = project(&pool, &viewer, "ws-a").await;
    let b = project(&pool, &viewer, "ws-b").await;

    // The client groups by the project's root, so two workspaces reporting one
    // root would collapse into a single sidebar group.
    assert_ne!(a["workspaceRoot"], b["workspaceRoot"]);
    assert_eq!(a["title"], "ws-ws-a");
    assert_eq!(b["title"], "ws-ws-b");
}

/// The listing's widening half: a workspace the viewer cannot read is included
/// once something in the listing references it, and absent otherwise. Nothing
/// references a workspace yet — tasks are not listed — so this is asserted one
/// level down, at the query the listing is built from.
#[sqlx::test]
async fn an_unreadable_workspace_is_included_only_when_referenced(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());
    repo.insert_workspace(&make_workspace_row("ws-other", "user-2"))
        .await
        .unwrap();

    let unreferenced = repo
        .list_readable_including(&viewer("user-1"), &[])
        .await
        .unwrap();
    assert!(unreferenced.is_empty());

    let referenced = repo
        .list_readable_including(&viewer("user-1"), &["ws-other".to_string()])
        .await
        .unwrap();
    assert_eq!(
        referenced
            .iter()
            .map(|listed| &listed.workspace.id)
            .collect::<Vec<_>>(),
        vec!["ws-other"]
    );
    assert!(
        !referenced[0].readable,
        "the widened half must say it is the widened half, or the listing \
         above it cannot project it any differently"
    );
}

#[sqlx::test]
async fn a_referenced_workspace_that_was_deleted_stays_absent(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());
    let mut deleted = make_workspace_row("ws-gone", "user-2");
    deleted.deleted = true;
    repo.insert_workspace(&deleted).await.unwrap();

    let listed = repo
        .list_readable_including(&viewer("user-1"), &["ws-gone".to_string()])
        .await
        .unwrap();
    assert!(listed.is_empty());
}

/// The whole path a client takes: the env-gated router as it is mounted in
/// production, the session's identity, the database, and the JSON body.
#[sqlx::test]
async fn the_mounted_route_returns_the_viewers_projects(pool: PgPool) {
    std::env::set_var("T3_UI_RPC_ENABLED", "true");
    std::env::set_var("T3_UI_RPC_ALLOWED_ORIGINS", "http://localhost:5733");

    let mut ws = make_workspace_row("ws-http", USER_1);
    ws.name = "Checkout".into();
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&ws)
        .await
        .unwrap();
    seed_task(&pool, "t-http", USER_1, Some("ws-http")).await;

    let router = router_from_env(mounted_dependencies(&pool))
        .await
        .unwrap()
        .expect("the compatibility routes are gated on")
        .router;

    let response = router
        .oneshot(ui_rpc_authed_request(
            "GET",
            "/api/orchestration/shell",
            USER_1,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let body = body_json(response).await;
    let projects = body["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 1);
    assert_eq!(projects[0]["title"], "Checkout");
    assert_eq!(projects[0]["workspaceRoot"], "/moatless/workspaces/ws-http");

    let threads = body["threads"].as_array().unwrap();
    assert_eq!(threads.len(), 1, "the creator follows what they created");
    assert_eq!(threads[0]["id"], "t-http");
    assert_eq!(threads[0]["projectId"], "ws-http");
    assert_eq!(threads[0]["title"], "Test task t-http");
    assert_decodable_thread(&threads[0]);
}

#[sqlx::test]
async fn a_projects_scripts_are_separate_from_servers_and_setup(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());
    let mut ws = make_workspace_row("ws-a", "user-1");
    ws.setup_commands = Some(run_config(serde_json::json!(["bun install"])));
    ws.servers = Some(run_config(serde_json::json!([{
        "name": "dev",
        "port": 3000,
        "startCommands": ["bun run dev"],
        "default": true
    }])));
    ws.scripts = Some(run_config(
        json!([{ "name": "dev", "command": "bun run dev" }]),
    ));
    repo.insert_workspace(&ws).await.unwrap();

    let scripts = project(&pool, &viewer("user-1"), "ws-a").await["scripts"]
        .as_array()
        .unwrap()
        .clone();

    assert_eq!(scripts.len(), 1, "setup commands are not scripts");
    assert_eq!(scripts[0]["id"], "dev");
    assert_eq!(scripts[0]["name"], "dev");
    assert_eq!(scripts[0]["command"], "bun run dev");
    assert_eq!(scripts[0]["icon"], "play");
    // Scripts run only on request unless their declaration opts in.
    assert_eq!(scripts[0]["runOnWorktreeCreate"], false);
    assert!(
        scripts[0].get("previewUrl").is_none(),
        "a stored port is not a preview URL"
    );
}

/// `runOnWorktreeCreate` is the client's name for `run_on_start`, and the
/// backend acts on it: a flagged Script starts when the Sandbox reports Ready.
#[sqlx::test]
async fn a_script_that_runs_on_sandbox_start_is_projected_as_flagged(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());
    let mut ws = make_workspace_row("ws-a", "user-1");
    ws.scripts = Some(run_config(json!([
        { "name": "dev", "command": "bun run dev", "runOnStart": true },
        { "name": "test", "command": "bun test" }
    ])));
    repo.insert_workspace(&ws).await.unwrap();

    let scripts = project(&pool, &viewer("user-1"), "ws-a").await["scripts"]
        .as_array()
        .unwrap()
        .clone();

    assert_eq!(scripts[0]["id"], "dev");
    assert_eq!(scripts[0]["runOnWorktreeCreate"], true);
    assert_eq!(scripts[1]["id"], "test");
    assert_eq!(scripts[1]["runOnWorktreeCreate"], false);
}

/// Server and setup declarations do not become client-run Scripts.
#[sqlx::test]
async fn the_repositorys_server_config_does_not_create_scripts(pool: PgPool) {
    let mut ws = make_workspace_row("ws-vibe", "user-1");
    ws.name = "Moatless".into();
    ws.setup_commands = Some(run_config(serde_json::json!([
        "curl -fsSL https://bun.sh/install | bash",
        "bun install"
    ])));
    ws.servers = Some(run_config(serde_json::json!([
        {
            "name": "dev",
            "port": 3000,
            "default": true,
            "autoReload": true,
            "startCommands": ["bun run dev -- --host 0.0.0.0"],
            "startupTimeoutSeconds": 120
        },
        {
            "name": "backend",
            "label": "Shadow Backend",
            "port": 8080,
            "startCommands": ["bash scripts/shadow/up.sh"],
            "startupTimeoutSeconds": 600
        }
    ])));
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&ws)
        .await
        .unwrap();

    let scripts = project(&pool, &viewer("user-1"), "ws-vibe").await["scripts"]
        .as_array()
        .unwrap()
        .clone();

    assert!(
        scripts.is_empty(),
        "servers and setup commands are not scripts"
    );
}

// =========================================================================
// Which conversations a viewer sees
// =========================================================================

/// The listing's query, put in front of a real Postgres with nothing seeded.
///
/// Deliberately the first thing asserted about conversations. `sqlx::query_as`
/// without the macro is checked by nothing until a connection sees it, so a
/// wrong column name, a mis-numbered bind or a LATERAL that will not plan
/// reaches this file as a runtime error. Failing here says *the statement is
/// wrong*; failing in a seeded test with an empty result says nothing.
#[sqlx::test]
async fn postgres_accepts_the_conversation_query(pool: PgPool) {
    let repository = moatless_backend::t3code::conversations_over(&pool);
    for viewer in [viewer("user-1"), admin("root")] {
        let listed = repository
            .list_followed(&viewer)
            .await
            .unwrap_or_else(|error| {
                panic!("the conversation query was rejected: {error}\n\n{error:?}")
            });
        assert!(listed.is_empty(), "nothing is seeded");
    }
}

/// The production incident's exact failure shape, against a real Postgres:
/// one shell query blocked in the database, then another snapshot arriving at
/// the same backend replica.
///
/// The second request must be rejected before it acquires a pool connection,
/// and PostgreSQL itself must cancel the first query after five seconds. Before
/// the execution guard this test's second read waited for the pool timeout;
/// before the statement timeout the first read stayed blocked until the test's
/// outer deadline.
#[sqlx::test]
async fn a_blocked_full_listing_cannot_monopolize_the_pool(pool: PgPool) {
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("LOCK TABLE tasks IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *blocker)
        .await
        .unwrap();

    let listing_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    let repository = Arc::new(moatless_backend::t3code::conversations_over(&listing_pool));
    let running_repository = Arc::clone(&repository);
    let running =
        tokio::spawn(async move { running_repository.list_followed(&viewer("user-1")).await });

    // The first read has a connection immediately and blocks when it reaches
    // `tasks`. Give it time to take the repository's full-listing permit.
    tokio::time::sleep(Duration::from_millis(100)).await;

    let rejected_at = Instant::now();
    let rejected = repository
        .list_followed(&viewer("user-1"))
        .await
        .expect_err("a second full listing must not enter PostgreSQL");
    assert!(matches!(rejected, sqlx::Error::PoolTimedOut));
    assert!(
        rejected_at.elapsed() < Duration::from_secs(1),
        "overload was queued until a pool timeout instead of rejected"
    );

    let timed_out = tokio::time::timeout(Duration::from_secs(7), running)
        .await
        .expect("the database statement timeout did not fire")
        .expect("listing task panicked")
        .expect_err("the blocked listing unexpectedly completed");
    match timed_out {
        sqlx::Error::Database(error) => {
            assert_eq!(error.code().as_deref(), Some("57014"), "{error}");
        }
        other => panic!("expected PostgreSQL query cancellation, got {other:?}"),
    }

    blocker.rollback().await.unwrap();

    let listed = repository
        .list_followed(&viewer("user-1"))
        .await
        .expect("the timed-out connection must return to the pool");
    assert!(listed.is_empty());
}

/// Each per-conversation message lookup in the full listing has a purpose-built
/// partial index. Naming them here makes a dropped migration fail the database
/// tier instead of returning as a production query-plan regression.
#[sqlx::test]
async fn the_shell_message_lookups_have_dedicated_indexes(pool: PgPool) {
    let expected: HashSet<String> = [
        "task_messages_latest_user_created_idx",
        "task_messages_latest_assistant_idx",
        "task_messages_turn_assistant_model_idx",
        "task_messages_turn_requested_model_idx",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let names: HashSet<String> = sqlx::query_scalar(
        "SELECT indexname
         FROM pg_indexes
         WHERE schemaname = current_schema()
           AND indexname = ANY($1)",
    )
    .bind(expected.iter().cloned().collect::<Vec<_>>())
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .collect();

    assert_eq!(names, expected);
}

/// The one project every matrix conversation lives in. Its own readability is
/// not what these tests vary — the conversation's is.
const MATRIX_WS: &str = "ws-matrix";

/// One conversation per combination the two filters have to distinguish: who
/// owns it, whether it is public, whether it is user- or deployment-scoped,
/// and whether the viewer follows it.
///
/// Ids read `{owner}-{visibility}-{scope}-{follow}`, so a failing assertion
/// names the combination that broke.
async fn seed_conversation_matrix(pool: &PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row(MATRIX_WS, "user-1"))
        .await
        .unwrap();

    for (owner_label, owner) in [("own", "user-1"), ("other", "user-2")] {
        for (visibility_label, visibility) in [
            ("private", Visibility::Private),
            ("public", Visibility::Public),
        ] {
            for (scope_label, scope) in [("user", Scope::User), ("global", Scope::Global)] {
                for (follow_label, followed) in [("followed", true), ("unfollowed", false)] {
                    let id =
                        format!("{owner_label}-{visibility_label}-{scope_label}-{follow_label}");
                    let mut task = make_task_row(&id, owner);
                    task.workspace_id = Some(MATRIX_WS.to_string());
                    task.visibility = visibility;
                    task.scope = scope;
                    // `tasks_owner_scope_chk`: a deployment-scoped row has no
                    // owner. `created_by` still records who made it.
                    task.owner_user_id = match scope {
                        Scope::User => Some(owner.to_string()),
                        Scope::Global => None,
                    };
                    moatless_tasks::task::dao::store_over(pool)
                        .insert_task(&task)
                        .await
                        .unwrap();

                    // Creation auto-follows for the creator, so the viewer's
                    // follow is set both ways rather than only added.
                    if followed {
                        follow(pool, "user-1", &id).await;
                    } else {
                        unfollow(pool, "user-1", &id).await;
                    }
                }
            }
        }
    }
}

#[sqlx::test]
async fn a_viewer_sees_the_conversations_they_follow_and_may_still_read(pool: PgPool) {
    seed_conversation_matrix(&pool).await;

    // Every followed combination except the one the viewer may not read:
    // someone else's private, user-scoped conversation. Public and global
    // conversations they follow are listed whoever owns them.
    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        vec![
            "other-private-global-followed".to_string(),
            "other-public-global-followed".to_string(),
            "other-public-user-followed".to_string(),
            "own-private-global-followed".to_string(),
            "own-private-user-followed".to_string(),
            "own-public-global-followed".to_string(),
            "own-public-user-followed".to_string(),
        ]
    );
}

#[sqlx::test]
async fn an_unfollowed_conversation_is_absent_however_readable_it_is(pool: PgPool) {
    seed_conversation_matrix(&pool).await;

    let listed = thread_ids(&pool, &viewer("user-1")).await;
    for id in &listed {
        assert!(
            id.ends_with("-followed"),
            "{id} is in the listing without being followed"
        );
    }
    // Including the viewer's own, which they can both read and control.
    assert!(!listed.contains(&"own-public-user-unfollowed".to_string()));
    assert!(!listed.contains(&"own-private-user-unfollowed".to_string()));
}

/// An administrator's sidebar is what they follow, like everyone else's. This
/// is deliberately the opposite of the project half, where an administrator
/// does see every workspace in the deployment.
#[sqlx::test]
async fn an_administrator_sees_only_the_conversations_they_follow(pool: PgPool) {
    seed_conversation_matrix(&pool).await;

    let root = admin("root");
    assert!(
        thread_ids(&pool, &root).await.is_empty(),
        "an administrator who follows nothing has an empty sidebar"
    );

    // A conversation the administrator neither owns nor could reach through
    // scope — following it is the whole reason it appears.
    follow(&pool, "root", "other-private-user-unfollowed").await;
    assert_eq!(
        thread_ids(&pool, &root).await,
        vec!["other-private-user-unfollowed".to_string()]
    );
}

/// Following is durable; access is not. The follow row outlives the access
/// that produced it, and the listing has to notice.
#[sqlx::test]
async fn a_followed_conversation_the_viewer_may_no_longer_read_drops_out(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-handed-over", "user-1", Some("ws-a")).await;

    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-handed-over".to_string()]
    );

    // Ownership moves; the follow stays.
    sqlx::query("UPDATE tasks SET owner_user_id = $1, created_by = $1 WHERE task_id = $2")
        .bind("user-2")
        .bind("t-handed-over")
        .execute(&pool)
        .await
        .unwrap();
    let still_following: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM task_follows WHERE user_id = $1)")
            .bind("user-1")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(still_following, "the follow is what outlives the access");

    assert!(thread_ids(&pool, &viewer("user-1")).await.is_empty());
}

/// Writing to a conversation follows it — the same DAO call `send_message`
/// makes — which is what puts someone else's conversation in your sidebar.
#[sqlx::test]
async fn writing_to_a_conversation_puts_it_in_the_sidebar(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    let mut task = make_task_row("t-shared", "user-2");
    task.workspace_id = Some("ws-a".to_string());
    task.visibility = Visibility::Public;
    moatless_tasks::task::dao::store_over(&pool)
        .insert_task(&task)
        .await
        .unwrap();

    // Readable — it is public — but not yet the viewer's concern.
    assert!(thread_ids(&pool, &viewer("user-1")).await.is_empty());

    follow(&pool, "user-1", "t-shared").await;
    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-shared".to_string()]
    );
}

/// The listing's widening half, end to end: the conversation is readable, its
/// workspace is not, and the row still needs a project to hang under or the
/// client silently drops it.
#[sqlx::test]
async fn a_followed_conversation_in_an_unreadable_project_still_lists_under_it(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-theirs", "user-2"))
        .await
        .unwrap();
    let mut task = make_task_row("t-guest", "user-2");
    task.workspace_id = Some("ws-theirs".to_string());
    task.visibility = Visibility::Public;
    moatless_tasks::task::dao::store_over(&pool)
        .insert_task(&task)
        .await
        .unwrap();
    follow(&pool, "user-1", "t-guest").await;

    let viewer = viewer("user-1");
    let snapshot = snapshot(&pool, &viewer).await;

    let projects = snapshot["projects"].as_array().unwrap();
    assert_eq!(projects.len(), 1, "the referenced workspace is pulled in");
    assert_eq!(projects[0]["id"], "ws-theirs");
    assert_eq!(projects[0]["title"], "ws-ws-theirs");

    let threads = snapshot["threads"].as_array().unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0]["projectId"], "ws-theirs");
}

/// The widening is justified by names, and must not carry more than names.
///
/// Decision 5 admits a Workspace the viewer may **not** read into their listing
/// so that a conversation they follow has a group to hang under, and it prices
/// that as "the small disclosure": *"seeing the names gives a person the shape
/// of the deployment they work in"*. A server's start command is not part of
/// that shape — it carries paths, flags and whatever else was put in a server
/// config — so it stops at the read rule.
///
/// Referenced projects disclose Scripts only when the viewer can read the Workspace.
#[sqlx::test]
async fn a_project_the_viewer_may_not_read_discloses_no_script_commands(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());
    let servers = |command: &str| {
        json!([{
            "name": "dev",
            "label": "Dev server",
            "port": 3000,
            "startCommands": [command],
            "default": true
        }])
    };

    let mut mine = make_workspace_row("ws-own", "user-1");
    mine.servers = Some(run_config(servers("bun run dev")));
    mine.scripts = Some(run_config(
        json!([{ "name": "dev", "command": "bun run dev" }]),
    ));
    repo.insert_workspace(&mine).await.unwrap();

    let mut theirs = make_workspace_row("ws-theirs", "user-2");
    theirs.servers = Some(run_config(servers(
        "bun run secret-thing --token /etc/theirs",
    )));
    theirs.scripts = Some(run_config(json!([{
        "name": "dev",
        "command": "bun run secret-thing --token /etc/theirs"
    }])));
    repo.insert_workspace(&theirs).await.unwrap();

    // The one thing that pulls their private workspace into this viewer's
    // listing: a conversation they follow that runs in it.
    let mut task = make_task_row("t-guest", "user-2");
    task.workspace_id = Some("ws-theirs".to_string());
    task.visibility = Visibility::Public;
    moatless_tasks::task::dao::store_over(&pool)
        .insert_task(&task)
        .await
        .unwrap();
    follow(&pool, "user-1", "t-guest").await;

    let viewer = viewer("user-1");
    let snapshot = snapshot(&pool, &viewer).await;
    let project = |id: &str| {
        snapshot["projects"]
            .as_array()
            .unwrap()
            .iter()
            .find(|project| project["id"] == id)
            .unwrap_or_else(|| panic!("{id} is absent from the listing"))
            .clone()
    };

    // Their own workspace is untouched by this: the narrowing is one half of
    // the union, not the listing.
    let ours = project("ws-own");
    assert_eq!(ours["scripts"][0]["command"], "bun run dev");

    let theirs = project("ws-theirs");
    assert_eq!(
        theirs["scripts"],
        json!([]),
        "a workspace this viewer may not read disclosed its script commands"
    );
    // What the widening does justify, still there — the group renders, and the
    // conversation hanging under it is not silently dropped.
    assert_eq!(theirs["title"], "ws-ws-theirs");
    assert!(
        theirs["workspaceRoot"]
            .as_str()
            .unwrap()
            .contains("ws-theirs")
    );
    assert_eq!(
        snapshot["threads"][0]["projectId"], "ws-theirs",
        "the narrowing must not cost the row its group"
    );
    assert_decodable_item(&json!({ "kind": "snapshot", "snapshot": snapshot }));
}

/// T3 drops a conversation whose project is not in the same snapshot — no
/// error, nothing logged (`Sidebar.logic.ts:779`). Leaving it out here is what
/// makes the omission visible on this side of the wire.
#[sqlx::test]
async fn a_conversation_with_no_project_is_left_out_of_the_listing(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-placed", "user-1", Some("ws-a")).await;
    seed_task(&pool, "t-orphan", "user-1", Some("ws-a")).await;
    // Creation always resolves a workspace, so the unresolvable row has to be
    // made after the fact — as it exists in deployments predating that rule.
    sqlx::query("UPDATE tasks SET workspace_id = NULL WHERE task_id = $1")
        .bind("t-orphan")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-placed".to_string()]
    );
}

/// The third filter, and the one no access rule expresses: a sidebar is about
/// unfinished work, and a follow is durable, so without this every conversation
/// a person ever took part in stays on their screen for good.
///
/// `error` is left out with `closed` — the narrow reading of "open", and the
/// same one the Moatless sidebar applies by default. Nothing that is *running*
/// is hidden by it: a Task returns to `open` the moment its agent produces
/// anything.
#[sqlx::test]
async fn only_open_conversations_are_listed(pool: PgPool) {
    seed_live_project(&pool).await;
    for (task_id, status) in [
        ("t-open", TaskStatus::Open),
        ("t-closed", TaskStatus::Closed),
        ("t-errored", TaskStatus::Error),
    ] {
        let mut task = make_task_row(task_id, "user-1");
        task.workspace_id = Some("ws-live".to_string());
        task.status = status;
        moatless_tasks::task::dao::store_over(&pool)
            .insert_task(&task)
            .await
            .unwrap();
    }

    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-open".to_string()]
    );
    // Following is untouched by it — the row is filtered out of the listing,
    // not unfollowed — so reopening one puts it straight back.
    moatless_tasks::task::dao::store_over(&pool)
        .update_task(
            "t-closed",
            &TaskPatch {
                status: Some(TaskStatus::Open),
                ..TaskPatch::default()
            },
            &viewer("user-1"),
        )
        .await
        .unwrap();
    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-closed".to_string(), "t-open".to_string()]
    );
}

/// A row by id is not a listing of one: it answers about the conversation, not
/// about this viewer's involvement in it. So it reaches everything the two
/// listings between them leave out — closed work, and work followed by someone
/// else — which is what lets a client name a conversation it opened from a
/// link or a task tree.
///
/// It is not wider than *access*, and that is the line: the same read a link
/// resolves through, so a viewer is told about a conversation here exactly when
/// they could have opened it.
#[sqlx::test]
async fn a_listing_row_by_id_reaches_what_neither_listing_carries(pool: PgPool) {
    seed_live_project(&pool).await;
    for (task_id, status) in [
        ("t-open", TaskStatus::Open),
        ("t-closed", TaskStatus::Closed),
    ] {
        let mut task = make_task_row(task_id, "user-1");
        task.workspace_id = Some("ws-live".to_string());
        moatless_tasks::task::dao::store_over(&pool)
            .insert_task(&task)
            .await
            .unwrap();
        moatless_tasks::task::dao::store_over(&pool)
            .update_task(
                task_id,
                &TaskPatch {
                    status: Some(status),
                    ..TaskPatch::default()
                },
                &viewer("user-1"),
            )
            .await
            .unwrap();
    }
    seed_public_task(&pool, "t-unfollowed", "user-2", Some("ws-live")).await;
    unfollow(&pool, "user-1", "t-unfollowed").await;

    // What the sidebar shows, and what it leaves out.
    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-open".to_string()]
    );

    for task_id in ["t-open", "t-closed", "t-unfollowed"] {
        let row = thread_shell_row(&pool, &viewer("user-1"), task_id)
            .await
            .unwrap_or_else(|| panic!("{task_id} has no listing row"));
        assert_eq!(row["id"], task_id);
        // The listing's row, not a second shape that happens to carry an id.
        assert_decodable_thread(&row);
    }
    assert!(
        thread_shell_row(&pool, &viewer("user-1"), "t-closed")
            .await
            .unwrap()["archivedAt"]
            .is_string(),
        "a closed conversation's row says it is archived"
    );
}

// =========================================================================
// Linked pull requests
// =========================================================================

/// The connection every seeded binding hangs off, since `connection_id` is a
/// foreign key.
async fn seed_pr_connection(pool: &PgPool) {
    sqlx::query(
        r#"INSERT INTO adapter_connections
               (id, adapter_kind, connection_kind, external_account_id, created_by,
                scope, owner_user_id)
           VALUES ('conn-gh', 'github', 'org', 'acme', 'user-1', 'user', 'user-1')
           ON CONFLICT (id) DO NOTHING"#,
    )
    .execute(pool)
    .await
    .unwrap();
}

/// Bind a Task to `subject_key`, in the shape the outbound path writes: a
/// binding carries no status blob until a refresh fetches one, and `state_kind`
/// is what that refresh later fills in.
async fn seed_pr_binding(
    pool: &PgPool,
    binding_id: &str,
    task_id: &str,
    subject_key: &str,
    state_kind: Option<&str>,
) {
    sqlx::query(
        r#"INSERT INTO task_bindings
               (id, task_id, adapter_kind, subject_kind, subject_key, connection_id,
                destination_config, origin, external_state_kind)
           VALUES ($1, $2, 'github', 'github_pr', $3, 'conn-gh', '{}'::jsonb,
                   'connected_thread', $4)"#,
    )
    .bind(binding_id)
    .bind(task_id)
    .bind(subject_key)
    .bind(state_kind)
    .execute(pool)
    .await
    .unwrap();
}

/// The regression behind the surfaces showing nothing: a binding is written
/// with `external_status_raw` NULL and only filled in by a refresh the listing
/// cannot trigger, so a link conditioned on that blob never appeared for a pull
/// request the agent had just opened.
#[sqlx::test]
async fn a_thread_links_a_pull_request_whose_status_was_never_fetched(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-pr", "user-1", Some("ws-live")).await;
    seed_pr_connection(&pool).await;
    seed_pr_binding(&pool, "b-1", "t-pr", "acme/web#7", None).await;

    let row = thread(&pool, &viewer("user-1"), "t-pr").await;

    assert_eq!(row["linkedPullRequest"]["repository"], "acme/web");
    assert_eq!(row["linkedPullRequest"]["number"], 7);
    // Recovered from `subject_key`, with no blob to read it out of.
    assert_eq!(
        row["linkedPullRequest"]["url"],
        "https://github.com/acme/web/pull/7"
    );
    assert_eq!(row["linkedPullRequests"].as_array().unwrap().len(), 1);
}

#[sqlx::test]
async fn a_thread_links_every_bound_pull_request_live_first_then_newest(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-prs", "user-1", Some("ws-live")).await;
    seed_pr_connection(&pool).await;
    seed_pr_binding(&pool, "b-1", "t-prs", "acme/web#7", Some("open")).await;
    seed_pr_binding(&pool, "b-2", "t-prs", "acme/web#9", Some("merged")).await;
    seed_pr_binding(&pool, "b-3", "t-prs", "acme/api#8", None).await;

    let row = thread(&pool, &viewer("user-1"), "t-prs").await;
    let linked = row["linkedPullRequests"].as_array().unwrap();

    // Live before terminal, then highest number — so the merged #9 sorts last
    // and the unrefreshed #8 counts as live.
    let ordered: Vec<_> = linked
        .iter()
        .map(|pr| (pr["repository"].as_str().unwrap(), pr["number"].as_i64()))
        .collect();
    assert_eq!(
        ordered,
        vec![
            ("acme/api", Some(8)),
            ("acme/web", Some(7)),
            ("acme/web", Some(9)),
        ]
    );
    // The single reference every existing reader uses is the list's primary.
    assert_eq!(row["linkedPullRequest"], linked[0]);
}

/// The guard exists so a malformed key drops its own row: the `::bigint` cast
/// runs after the `WHERE`, so a key that reached it with a non-numeric second
/// field would fail the whole listing rather than one binding.
#[sqlx::test]
async fn a_binding_whose_key_is_not_owner_repo_number_drops_without_failing_the_listing(
    pool: PgPool,
) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-odd", "user-1", Some("ws-live")).await;
    seed_pr_connection(&pool).await;
    seed_pr_binding(&pool, "b-1", "t-odd", "acme/web#7", None).await;
    seed_pr_binding(&pool, "b-2", "t-odd", "acme#web#7", None).await;
    seed_pr_binding(&pool, "b-3", "t-odd", "acme/web#none", None).await;

    let row = thread(&pool, &viewer("user-1"), "t-odd").await;
    let linked = row["linkedPullRequests"].as_array().unwrap();

    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0]["repository"], "acme/web");
    assert_eq!(linked[0]["number"], 7);
}

#[sqlx::test]
async fn a_thread_bound_to_no_pull_request_links_none(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-bare", "user-1", Some("ws-live")).await;

    let row = thread(&pool, &viewer("user-1"), "t-bare").await;

    assert!(row["linkedPullRequest"].is_null());
    assert!(row["linkedPullRequests"].as_array().unwrap().is_empty());
}

/// Absence is one answer with one meaning, and the reader cannot tell which of
/// its causes applies — which is the point, because each of them is a different
/// fact about somebody else's work.
#[sqlx::test]
async fn a_listing_row_for_a_conversation_out_of_reach_is_absent(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-private", "user-2", Some("ws-live")).await;
    let mut errored = make_task_row("t-errored", "user-1");
    errored.workspace_id = Some("ws-live".to_string());
    errored.status = TaskStatus::Error;
    moatless_tasks::task::dao::store_over(&pool)
        .insert_task(&errored)
        .await
        .unwrap();

    for task_id in ["t-private", "t-errored", "t-never-existed"] {
        assert!(
            thread_shell_row(&pool, &viewer("user-1"), task_id)
                .await
                .is_none(),
            "{task_id} disclosed a listing row"
        );
    }
}

/// The two halves are one partition of what a person follows: archiving is
/// closing, so a conversation is in exactly one of them and moves between them
/// by changing status and nothing else.
///
/// `archivedAt` is what tells the client which half a row came from, and it is
/// maintained by the write that sets the status — so closing stamps it and
/// reopening clears it, with no separate call to forget.
#[sqlx::test]
async fn a_closed_conversation_moves_to_the_archived_listing(pool: PgPool) {
    seed_live_project(&pool).await;
    for (task_id, status) in [
        ("t-open", TaskStatus::Open),
        ("t-closed", TaskStatus::Closed),
        ("t-errored", TaskStatus::Error),
    ] {
        let mut task = make_task_row(task_id, "user-1");
        task.workspace_id = Some("ws-live".to_string());
        task.status = status;
        moatless_tasks::task::dao::store_over(&pool)
            .insert_task(&task)
            .await
            .unwrap();
    }

    // `error` belongs to neither half: it is not open, and it was not put
    // away. Its conversation is unfinished work whose agent fell over.
    assert_eq!(
        archived_thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-closed".to_string()]
    );

    let before: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&pool)
        .await
        .unwrap();
    moatless_tasks::task::dao::store_over(&pool)
        .update_task(
            "t-open",
            &TaskPatch {
                status: Some(TaskStatus::Closed),
                ..TaskPatch::default()
            },
            &viewer("user-1"),
        )
        .await
        .unwrap();

    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        Vec::<String>::new(),
        "a closed conversation is off the sidebar"
    );
    let archived = archived_snapshot(&pool, &viewer("user-1")).await;
    let closed = archived["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["id"] == "t-open")
        .expect("the conversation just closed is in the archive");
    assert!(
        parse(&closed["archivedAt"]) >= before.trunc_subsecs(3),
        "archivedAt is the time it was closed, got {}",
        closed["archivedAt"]
    );

    moatless_tasks::task::dao::store_over(&pool)
        .update_task(
            "t-open",
            &TaskPatch {
                status: Some(TaskStatus::Open),
                ..TaskPatch::default()
            },
            &viewer("user-1"),
        )
        .await
        .unwrap();

    assert_eq!(
        thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-open".to_string()]
    );
    assert_eq!(
        archived_thread_ids(&pool, &viewer("user-1")).await,
        vec!["t-closed".to_string()],
        "reopening takes the conversation out of the archive"
    );
    assert_eq!(
        thread(&pool, &viewer("user-1"), "t-open").await["archivedAt"],
        Value::Null,
        "reopening clears the close time"
    );
}

/// Opening a conversation by id is filtered by **access**, and the sidebar by
/// Follow, so a link to a Task this viewer may read resolves whether or not it
/// is on their sidebar. A person handed a URL for work they are allowed to see
/// opens it; being told it does not exist is the thing this rules out.
///
/// The two halves of that are asserted together, because the widening is only
/// correct while the second one holds: the conversation opens, and the sidebar
/// stays exactly as narrow as it was.
#[sqlx::test]
async fn a_readable_conversation_opens_by_id_without_being_followed(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_public_task(&pool, "t-theirs", "user-2", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let snapshot = thread_snapshot(
        &moatless_backend::t3code::conversations_over(&pool),
        &moatless_backend::t3code::conversations_over(&pool),
        &Deployment {
            model_defaults: &defaults(),
            frontend_url: "http://localhost:3000",
        },
        &viewer,
        "t-theirs",
    )
    .await
    .unwrap();

    assert!(
        snapshot.is_some(),
        "a public conversation this viewer may read opens from its id"
    );
    assert_eq!(
        thread_ids(&pool, &viewer).await,
        Vec::<String>::new(),
        "and opening it does not put it on their sidebar; writing to it is what would"
    );
}

/// Access is the whole of the widening. A Task this viewer may not read answers
/// exactly as it did when the follow rule was the one refusing it — one answer
/// for every way a conversation can be absent, so which one it was stays
/// undisclosed.
#[sqlx::test]
async fn an_unreadable_conversation_still_does_not_open_by_id(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-private", "user-2", Some("ws-live")).await;

    let snapshot = thread_snapshot(
        &moatless_backend::t3code::conversations_over(&pool),
        &moatless_backend::t3code::conversations_over(&pool),
        &Deployment {
            model_defaults: &defaults(),
            frontend_url: "http://localhost:3000",
        },
        &viewer("user-1"),
        "t-private",
    )
    .await
    .unwrap();

    assert!(
        snapshot.is_none(),
        "a private conversation belonging to somebody else stays absent"
    );
}

/// The archive is filtered by the same follow rule as the sidebar. A
/// conversation somebody else closed is not in this viewer's archive unless
/// they were involved in it.
#[sqlx::test]
async fn an_unfollowed_closed_conversation_is_absent_from_the_archive(pool: PgPool) {
    seed_live_project(&pool).await;
    let mut task = make_task_row("t-theirs", "user-1");
    task.workspace_id = Some("ws-live".to_string());
    task.status = TaskStatus::Closed;
    moatless_tasks::task::dao::store_over(&pool)
        .insert_task(&task)
        .await
        .unwrap();
    unfollow(&pool, "user-1", "t-theirs").await;

    assert!(
        archived_thread_ids(&pool, &viewer("user-1"))
            .await
            .is_empty()
    );
}

/// A workspace that was deleted is not listable, so a conversation still
/// pointing at it has no project either.
#[sqlx::test]
async fn a_conversation_whose_project_was_deleted_is_left_out(pool: PgPool) {
    let mut deleted = make_workspace_row("ws-gone", "user-1");
    deleted.deleted = true;
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&deleted)
        .await
        .unwrap();
    seed_task(&pool, "t-stranded", "user-1", Some("ws-gone")).await;

    assert!(thread_ids(&pool, &viewer("user-1")).await.is_empty());
}

// =========================================================================
// What one conversation row says
// =========================================================================

#[sqlx::test]
async fn two_followers_receive_their_own_settlement_state(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_public_task(&pool, "t-shared-state", "user-1", Some("ws-a")).await;
    follow(&pool, "user-2", "t-shared-state").await;
    moatless_tasks::task::dao::store_over(&pool)
        .settle_task("user-1", "t-shared-state", Utc::now())
        .await
        .unwrap();

    assert_eq!(
        thread(&pool, &viewer("user-1"), "t-shared-state").await["settledOverride"],
        json!("settled")
    );
    assert_eq!(
        thread(&pool, &viewer("user-2"), "t-shared-state").await["settledOverride"],
        Value::Null
    );
}

#[sqlx::test]
async fn a_user_message_clears_settle_for_all_followers_and_snooze_only_for_sender(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_public_task(&pool, "t-message-wake", "user-1", Some("ws-a")).await;
    follow(&pool, "user-2", "t-message-wake").await;
    set_settled_and_snoozed(&pool, "user-1", "t-message-wake").await;
    set_settled_and_snoozed(&pool, "user-2", "t-message-wake").await;

    let state = test_app_state(pool.clone());
    let sender_scope = viewer("user-1");
    message_service::send_user_message(
        state.tasks.as_ref(),
        state.message_send.as_ref(),
        state.sandbox_waker.as_ref(),
        &message_service::Sender {
            user_id: "user-1",
            scope: &sender_scope,
            is_agent_context: false,
        },
        "t-message-wake",
        message_service::UserMessage {
            text: "wake it".to_string(),
            resume: true,
            ..Default::default()
        },
    )
    .await
    .expect("message is accepted");

    let sender = follower_state(&pool, "user-1", "t-message-wake").await;
    let colleague = follower_state(&pool, "user-2", "t-message-wake").await;
    assert!(sender.0.is_none(), "sender settle was not cleared");
    assert!(sender.2.is_none(), "sender snooze was not cleared");
    assert!(colleague.0.is_none(), "colleague settle was not cleared");
    assert!(colleague.2.is_some(), "colleague snooze was cleared");
}

#[sqlx::test]
async fn turn_start_clears_settle_for_every_follower_and_leaves_snooze(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_public_task(&pool, "t-turn-wake", "user-1", Some("ws-a")).await;
    follow(&pool, "user-2", "t-turn-wake").await;
    set_settled_and_snoozed(&pool, "user-1", "t-turn-wake").await;
    set_settled_and_snoozed(&pool, "user-2", "t-turn-wake").await;

    start_turn(&pool, "t-turn-wake", "run-1").await;

    for user in ["user-1", "user-2"] {
        let state = follower_state(&pool, user, "t-turn-wake").await;
        assert!(state.0.is_none(), "{user} settle was not cleared");
        assert!(state.2.is_some(), "{user} snooze was cleared");
    }
}

#[sqlx::test]
async fn ask_user_question_clears_settle_for_every_follower_and_leaves_snooze(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_public_task(&pool, "t-question-wake", "user-1", Some("ws-a")).await;
    follow(&pool, "user-2", "t-question-wake").await;
    set_settled_and_snoozed(&pool, "user-1", "t-question-wake").await;
    set_settled_and_snoozed(&pool, "user-2", "t-question-wake").await;

    let mut asking = make_message_row("t-question-wake", "m-question-wake");
    asking.message_type = MessageType::Assistant;
    asking.user_type = "system".into();
    asking.turn_number = Some(1);
    let question = conversation_write(
        "t-question-wake",
        "m-question-wake",
        MessageType::Assistant,
        &json!([{"type": "tool_use", "id": "tu-wake", "name": "AskUserQuestion", "input": {}}]),
    );
    PgMessageDao(pool.clone())
        .append(&asking, &[], &question)
        .await
        .unwrap();

    for user in ["user-1", "user-2"] {
        let state = follower_state(&pool, user, "t-question-wake").await;
        assert!(state.0.is_none(), "{user} settle was not cleared");
        assert!(state.2.is_some(), "{user} snooze was cleared");
    }
}

#[sqlx::test]
async fn completed_turn_clears_no_settlement_or_snooze_state(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_public_task(&pool, "t-complete-keeps-state", "user-1", Some("ws-a")).await;
    follow(&pool, "user-2", "t-complete-keeps-state").await;
    let turn = start_turn(&pool, "t-complete-keeps-state", "run-1").await;
    set_settled_and_snoozed(&pool, "user-1", "t-complete-keeps-state").await;
    set_settled_and_snoozed(&pool, "user-2", "t-complete-keeps-state").await;

    ingress(&pool, "t-complete-keeps-state", turn, "agent", "completed").await;

    for user in ["user-1", "user-2"] {
        let state = follower_state(&pool, user, "t-complete-keeps-state").await;
        assert_eq!(state.0.as_deref(), Some("settled"), "{user}");
        assert!(state.1.is_some(), "{user} settled_at was cleared");
        assert!(state.2.is_some(), "{user} snooze was cleared");
        assert!(state.3.is_some(), "{user} snoozed_at was cleared");
    }
}

/// The activity timestamp counts messages a person wrote. `last_message_at`,
/// the nearest stored value, also advances when the agent writes — a sidebar
/// sorted or badged on it reorders itself while nobody is talking.
#[sqlx::test]
async fn the_activity_timestamp_does_not_move_when_the_agent_writes(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-chat", "user-1", Some("ws-a")).await;
    seed_turn(&pool, "t-chat", 1, "active").await;
    seed_user_message(&pool, "t-chat", "m-person").await;

    let viewer = viewer("user-1");
    let after_person = thread(&pool, &viewer, "t-chat").await["latestUserMessageAt"].clone();
    assert!(
        !after_person.is_null(),
        "a person wrote, so there is a time"
    );

    seed_agent_message(&pool, "t-chat", "m-agent-1", 1, Some("claude-opus-4-5")).await;
    seed_agent_message(&pool, "t-chat", "m-agent-2", 1, Some("claude-opus-4-5")).await;
    sqlx::query("UPDATE tasks SET last_message_at = $1 WHERE task_id = $2")
        .bind(Utc::now().to_rfc3339())
        .bind("t-chat")
        .execute(&pool)
        .await
        .unwrap();

    let after_agent = thread(&pool, &viewer, "t-chat").await["latestUserMessageAt"].clone();
    assert_eq!(
        after_agent, after_person,
        "the agent's messages moved the activity timestamp"
    );

    // And it really is the person's message time, not the row's. Compared at
    // the wire's precision: `timestamptz` keeps microseconds and every instant
    // on this wire is RFC 3339 with milliseconds, so the stored value has to be
    // truncated the same way the projection truncates it before they can be
    // equal at all.
    let wrote_at: DateTime<Utc> =
        sqlx::query_scalar("SELECT created_at FROM task_messages WHERE uuid = $1")
            .bind("m-person")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(parse(&after_person), wrote_at.trunc_subsecs(3));
}

/// A conversation that has run: a turn, a sandbox, a person's message and the
/// agent's reply. Every derived value the row carries comes from one of them.
#[sqlx::test]
async fn a_conversation_that_has_run_reports_its_turn_session_and_model(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-live", "user-1", Some("ws-a")).await;
    seed_turn(&pool, "t-live", 1, "completed").await;
    seed_turn(&pool, "t-live", 2, "active").await;
    seed_sandbox(&pool, "t-live", "running", true).await;
    seed_user_message(&pool, "t-live", "m-1").await;
    seed_agent_message(&pool, "t-live", "m-2", 2, Some("claude-opus-4-5-20260101")).await;

    let thread = thread(&pool, &viewer("user-1"), "t-live").await;
    assert_decodable_thread(&thread);

    // The newest turn, not the first.
    assert_eq!(thread["latestTurn"]["turnId"], "t-live:2");
    assert_eq!(thread["latestTurn"]["state"], "running");
    // An open turn is the session's active turn.
    assert_eq!(thread["session"]["activeTurnId"], "t-live:2");
    assert_eq!(thread["session"]["status"], "running");
    assert_eq!(thread["session"]["providerInstanceId"], "claudeAgent");
    // What the agent said it was running beats any configured default.
    assert_eq!(
        thread["modelSelection"]["model"],
        "claude-opus-4-5-20260101"
    );
    assert_eq!(thread["modelSelection"]["instanceId"], "claudeAgent");
    // Distinct per conversation, because the client sends it straight back as
    // the `cwd` of every file request it makes for this thread.
    assert_eq!(thread["worktreePath"], "/moatless/tasks/t-live");
    assert_eq!(thread["runtimeMode"], "full-access");
    assert_eq!(thread["hasPendingApprovals"], false);
}

/// Nothing has run: no turn, no sandbox, no message. Every derived value is
/// null and the row still has to decode, because this is what a conversation
/// looks like for its whole first minute.
#[sqlx::test]
async fn a_conversation_that_never_ran_still_decodes(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-fresh", "user-1", Some("ws-a")).await;

    let thread = thread(&pool, &viewer("user-1"), "t-fresh").await;
    assert_decodable_thread(&thread);

    assert!(thread["latestTurn"].is_null());
    assert_eq!(thread["session"]["status"], "stopped");
    assert!(thread["session"]["activeTurnId"].is_null());
    assert!(thread["latestUserMessageAt"].is_null());
    assert_eq!(thread["worktreePath"], "/moatless/tasks/t-fresh");
    // The deployment's configured default is the answer until something runs.
    assert_eq!(thread["modelSelection"]["model"], "claude-opus-4-5");
}

/// The agent's model default is per agent kind, and the provider instance the
/// client routes on comes from the same place.
#[sqlx::test]
async fn a_codex_conversation_reports_the_codex_instance_and_default(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    let mut task = make_task_row("t-codex", "user-1");
    task.workspace_id = Some("ws-a".to_string());
    task.agent_type = AgentType(moatless_tasks::agent::catalog::CODEX.id.to_string());
    moatless_tasks::task::dao::store_over(&pool)
        .insert_task(&task)
        .await
        .unwrap();
    seed_sandbox(&pool, "t-codex", "running", true).await;

    let thread = thread(&pool, &viewer("user-1"), "t-codex").await;
    assert_decodable_thread(&thread);
    assert_eq!(thread["modelSelection"]["instanceId"], "codex");
    assert_eq!(thread["modelSelection"]["model"], "gpt-5.6-sol");
    assert_eq!(thread["session"]["providerInstanceId"], "codex");
}

/// An unanswered question from the agent is what "waiting on a person" means
/// here; an answered one is not.
#[sqlx::test]
async fn an_unanswered_question_is_pending_user_input(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-asking", "user-1", Some("ws-a")).await;
    seed_turn(&pool, "t-asking", 1, "active").await;

    let mut asking = make_message_row("t-asking", "m-ask");
    asking.message_type = MessageType::Assistant;
    asking.user_type = "system".into();
    asking.turn_number = Some(1);
    asking.message_text = Some("Which one?".into());
    let question = map_content(
        BlockContext {
            task_id: "t-asking",
            message_uuid: "m-ask",
            parent_uuid: None,
            message_type: MessageType::Assistant,
            block_index: 1,
        },
        &json!([{"type": "tool_use", "id": "tu-1", "name": "AskUserQuestion", "input": {}}]),
    );
    PgMessageDao(pool.clone())
        .append(&asking, &[], &question)
        .await
        .unwrap();

    let viewer = viewer("user-1");
    let waiting = thread(&pool, &viewer, "t-asking").await;
    assert_eq!(waiting["hasPendingUserInput"], true);
    assert_eq!(waiting["hasActionableProposedPlan"], false);

    // The answer settles the call the question wrote. There is no second
    // Message carrying a `tool_result` any more, so the flag has to come off
    // the activity's own status.
    let answer = map_content(
        BlockContext {
            task_id: "t-asking",
            message_uuid: "m-answer",
            parent_uuid: None,
            message_type: MessageType::User,
            block_index: 0,
        },
        &json!([{"type": "tool_result", "tool_use_id": "tu-1", "content": "the first one"}]),
    );
    PgMessageDao(pool.clone())
        .append_activities("t-asking", "m-ask", &answer)
        .await
        .unwrap();

    assert_eq!(
        thread(&pool, &viewer, "t-asking").await["hasPendingUserInput"],
        false,
        "the question was answered"
    );
}

/// An unanswered question the agent then called past stops counting: the scan
/// reads only the newest turn that did anything, which is the narrow reading of
/// "waiting" — the agent has moved on.
#[sqlx::test]
async fn a_question_the_agent_called_past_is_no_longer_pending(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-moved-on", "user-1", Some("ws-a")).await;
    seed_turn(&pool, "t-moved-on", 1, "active").await;

    let mut asking = make_message_row("t-moved-on", "m-ask");
    asking.message_type = MessageType::Assistant;
    asking.user_type = "system".into();
    asking.turn_number = Some(1);
    asking.message_text = Some("Which one?".into());
    let question = map_content(
        BlockContext {
            task_id: "t-moved-on",
            message_uuid: "m-ask",
            parent_uuid: None,
            message_type: MessageType::Assistant,
            block_index: 1,
        },
        &json!([{"type": "tool_use", "id": "tu-1", "name": "AskUserQuestion", "input": {}}]),
    );
    PgMessageDao(pool.clone())
        .append(&asking, &[], &question)
        .await
        .unwrap();

    let viewer = viewer("user-1");
    assert_eq!(
        thread(&pool, &viewer, "t-moved-on").await["hasPendingUserInput"],
        true
    );

    // A later turn that called a tool and said nothing at all — no Message row.
    let moved_on = map_content(
        BlockContext {
            task_id: "t-moved-on",
            message_uuid: "m-next",
            parent_uuid: None,
            message_type: MessageType::Assistant,
            block_index: 0,
        },
        &json!([{"type": "tool_use", "id": "tu-2", "name": "Bash", "input": {}}]),
    );
    PgMessageDao(pool.clone())
        .append_activities("t-moved-on", "m-next", &moved_on)
        .await
        .unwrap();

    let thread = thread(&pool, &viewer, "t-moved-on").await;
    assert_decodable_thread(&thread);
    assert_eq!(
        thread["hasPendingUserInput"], false,
        "the question is not on the newest turn that did anything"
    );
}

/// A conversation with no name still needs a title: the contract refuses an
/// empty string, so an unnamed conversation shows the "New thread" placeholder
/// until it is auto-titled.
#[sqlx::test]
async fn a_conversation_with_no_name_falls_back_to_new_thread(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-a", "user-1"))
        .await
        .unwrap();
    let mut task = make_task_row("t-nameless", "user-1");
    task.workspace_id = Some("ws-a".to_string());
    task.name = None;
    moatless_tasks::task::dao::store_over(&pool)
        .insert_task(&task)
        .await
        .unwrap();

    let thread = thread(&pool, &viewer("user-1"), "t-nameless").await;
    assert_decodable_thread(&thread);
    assert_eq!(thread["title"], "New thread");
}

/// Every row in a listing built from every combination the filters distinguish
/// has to decode — one bad row is not one missing conversation, it is a blank
/// sidebar.
#[sqlx::test]
async fn every_listed_conversation_decodes(pool: PgPool) {
    seed_conversation_matrix(&pool).await;
    seed_turn(&pool, "own-private-user-followed", 1, "error").await;
    seed_sandbox(&pool, "own-private-user-followed", "removed", false).await;
    seed_turn(&pool, "own-public-global-followed", 1, "stopped").await;
    seed_sandbox(&pool, "own-public-global-followed", "stopped", true).await;
    seed_user_message(&pool, "other-public-user-followed", "m-1").await;

    let snapshot = snapshot(&pool, &viewer("user-1")).await;
    let threads = snapshot["threads"].as_array().unwrap();
    assert_eq!(threads.len(), 7);
    for thread in threads {
        assert_decodable_thread(thread);
        assert_eq!(
            thread["projectId"], MATRIX_WS,
            "every row hangs under a project in the same snapshot"
        );
    }
}

// =========================================================================
// Addressing a conversation's files
// =========================================================================
//
// The client asks for files by the `cwd` it holds, and the only `cwd` it has
// for a conversation is the `worktreePath` this module put in the listing. So
// the two have to be each other's inverse, and the round trip is only real
// against a database: the reverse direction re-reads the conversation through
// the same follow and access filters the listing applies, which are SQL.
//
// What a Sandbox answers is a gRPC round trip against a pod, which no test here
// has. These stop where that begins — at the point the request has been
// addressed and authorized and is about to leave for the Sandbox — because
// everything before it is this module's and everything after it is not.

/// The `worktreePath` in a viewer's listing is the `cwd` that reaches that same
/// conversation's Sandbox, and only for a viewer whose listing contains it.
#[sqlx::test]
async fn the_listings_worktree_path_addresses_that_conversations_files(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-files", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-files", "user-1", Some("ws-files")).await;
    seed_sandbox(&pool, "t-files", "running", true).await;

    let owner = viewer("user-1");
    let cwd = thread(&pool, &owner, "t-files").await["worktreePath"]
        .as_str()
        .expect("a running sandbox reports a worktree")
        .to_string();

    // Reached the Sandbox: the stub is the only thing that refused it.
    let error = files::list_entries(
        &moatless_backend::t3code::conversations_over(&pool),
        Sources {
            sandboxes: &NoSandboxes,
            snapshots: None,
        },
        &owner,
        &cwd,
    )
    .await
    .expect_err("the stub connector has no sandbox to answer with");
    assert_eq!(
        error.message(),
        "The sandbox for this conversation is not running.",
        "the cwd resolved to the conversation and the viewer was allowed it"
    );

    // Same path, a viewer whose listing does not contain the conversation. A
    // `cwd` is a string the client sends, so this is the only thing standing
    // between a guessed Task id and someone else's repository.
    let outsider = ScopeContext {
        user_id: "user-2".into(),
        is_admin: false,
    };
    let error = files::read_file(
        &moatless_backend::t3code::conversations_over(&pool),
        Sources {
            sandboxes: &NoSandboxes,
            snapshots: None,
        },
        &outsider,
        &cwd,
        "README.md",
    )
    .await
    .expect_err("a conversation they do not follow is not theirs to read");
    assert_eq!(
        error.message(),
        "This conversation's workspace is not available.",
        "and it must not say whether the conversation exists"
    );
}

/// A conversation with no Sandbox reports no worktree, so the client falls back
/// to the project's root — and that has to come back as the state it is rather
/// than as a path nothing recognizes.
#[sqlx::test]
async fn a_project_root_is_answered_as_a_conversation_with_no_sandbox(pool: PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-bare", "user-1"))
        .await
        .unwrap();
    seed_task(&pool, "t-bare", "user-1", Some("ws-bare")).await;
    sqlx::query("UPDATE tasks SET sandbox_id = NULL WHERE task_id = $1")
        .bind("t-bare")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM sandboxes WHERE sandbox_id = $1")
        .bind("t-bare")
        .execute(&pool)
        .await
        .unwrap();

    let owner = viewer("user-1");
    let listing = snapshot(&pool, &owner).await;
    assert!(
        listing["threads"][0]["worktreePath"].is_null(),
        "no sandbox was ever asked for"
    );
    let cwd = listing["projects"][0]["workspaceRoot"]
        .as_str()
        .unwrap()
        .to_string();

    let error = files::list_entries(
        &moatless_backend::t3code::conversations_over(&pool),
        Sources {
            sandboxes: &NoSandboxes,
            snapshots: None,
        },
        &owner,
        &cwd,
    )
    .await
    .expect_err("a project has no files of its own");
    assert_eq!(
        error.message(),
        "This conversation has no sandbox, so it has no workspace yet."
    );
}

// =========================================================================
// Delivery: a watching client sees a turn change
// =========================================================================
//
// The snapshot is a listing read at a position; everything that happens after
// it reaches the client as stream events off the change feed. These tests drive
// the real writers — the turn-number allocator and the gRPC ingress — rather
// than inserting change records by hand, because a record written by a test
// helper proves only that the helper works. What is asserted is what a client
// watching the feed is handed.

/// The `thread-upserted` member of `OrchestrationShellStreamEvent`
/// (`orchestration.ts:449-453`). Three keys, all required, none optional — an
/// undeclared one fails the union's decode and the client applies nothing.
const THREAD_UPSERTED_REQUIRED: &[&str] = &["kind", "sequence", "thread"];

/// One delivered event, checked against the contract the way the whole row is:
/// keys first, then the nested thread, then `NonNegativeInt` on the position.
fn assert_decodable_upsert(event: &Value) {
    assert_keys(event, "thread-upserted", THREAD_UPSERTED_REQUIRED, &[]);
    assert_eq!(event["kind"], "thread-upserted");
    let sequence = event["sequence"]
        .as_i64()
        .expect("thread-upserted.sequence must be an integer");
    assert!(sequence >= 0, "`NonNegativeInt` refuses {sequence}");
    assert_decodable_thread(&event["thread"]);
}

/// The project the delivery tests run in. Owned by `user-1`, so `user-1` reads
/// it and follows what they create in it.
async fn seed_live_project(pool: &PgPool) {
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-live", "user-1"))
        .await
        .unwrap();
}

/// A client that has read the snapshot and is now watching — exactly the state
/// `subscribeShell` leaves behind, with the feed resuming from the position the
/// snapshot was read at.
async fn watching(pool: &PgPool, viewer: &ScopeContext) -> ShellFeed {
    let snapshot = shell_snapshot(
        &moatless_backend::t3code::conversations_over(pool),
        &moatless_backend::t3code::conversations_over(pool),
        &defaults(),
        viewer,
    )
    .await
    .unwrap();
    // Seeded with what the snapshot put in front of them, because that is the
    // only set a removal may ever name. A feed told nothing about what its
    // client holds cannot take anything back — see `ShellFeed`.
    let (threads, projects) = (snapshot.thread_ids(), snapshot.project_ids());
    ShellFeed::resuming_from(snapshot.identity, snapshot.sequence).holding(threads, projects)
}

/// One poll, as the subscription makes it.
async fn delivered(pool: &PgPool, feed: &mut ShellFeed, viewer: &ScopeContext) -> Vec<Value> {
    feed.poll(
        &moatless_backend::t3code::conversations_over(pool),
        &moatless_backend::t3code::conversations_over(pool),
        &defaults(),
        viewer,
    )
    .await
    .unwrap()
}

/// Exactly one delivered row, decoded — the shape every acceptance criterion
/// here is stated in ("its row moves", not "some events arrive").
async fn one_row(pool: &PgPool, feed: &mut ShellFeed, viewer: &ScopeContext) -> Value {
    let items = delivered(pool, feed, viewer).await;
    assert_eq!(
        items.len(),
        1,
        "expected exactly one delivered row, got {items:#?}"
    );
    assert_decodable_upsert(&items[0]);
    items[0].clone()
}

/// Start a turn the way a run does: the allocator, on the run's first signal.
async fn start_turn(pool: &PgPool, task_id: &str, run_id: &str) -> i32 {
    PgTaskTurnDao(pool.clone())
        .resolve_turn_for_run(task_id, run_id)
        .await
        .unwrap()
}

/// Apply a state-bearing event through the gRPC ingress writer, classified by
/// the same function the ingress itself uses — so a `(type, sub_type)` pair
/// that stops bearing turn state fails here rather than quietly delivering
/// nothing.
async fn ingress(pool: &PgPool, task_id: &str, turn_number: i32, event_type: &str, sub_type: &str) {
    let transition = classify_turn_transition(event_type, sub_type)
        .unwrap_or_else(|| panic!("{event_type}.{sub_type} bears no turn state"));
    PgTaskTurnDao(pool.clone())
        .apply_ingress_event(&TurnStateEvent {
            event_id: format!("ev-{task_id}-{turn_number}-{event_type}-{sub_type}"),
            task_id: task_id.to_string(),
            event_type: event_type.to_string(),
            sub_type: sub_type.to_string(),
            is_error: sub_type == "error",
            agent_type: Some("claude_code".to_string()),
            turn_number,
            payload: json!({}),
            timestamp: Utc::now().to_rfc3339(),
            transition,
        })
        .await
        .unwrap();
}

/// How many changes were recorded past a position. Used to show a test's
/// premise held: a coalescing assertion means nothing if only one change was
/// ever recorded, and a filtering assertion means nothing if the change the
/// viewer should not see was never written.
async fn recorded_after(pool: &PgPool, after: i64) -> usize {
    moatless_backend::t3code::conversations_over(pool)
        .listing_changes_after(after, 100)
        .await
        .unwrap()
        .len()
}

#[sqlx::test]
async fn starting_a_turn_moves_the_row_in_a_watching_client(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let mut feed = watching(&pool, &viewer).await;

    // Nothing has happened yet, so the client is told nothing.
    assert!(delivered(&pool, &mut feed, &viewer).await.is_empty());

    start_turn(&pool, "t-live", "run-1").await;

    let event = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(event["thread"]["id"], "t-live");
    assert_eq!(event["thread"]["projectId"], "ws-live");
    assert_eq!(
        event["thread"]["latestTurn"]["state"], "running",
        "a turn that just started is running"
    );
}

#[sqlx::test]
async fn every_way_a_turn_ends_moves_the_row(pool: PgPool) {
    seed_live_project(&pool).await;

    // Five Moatless endings onto T3's four states. `stopped` is a person
    // cutting a turn short, which T3 calls `interrupted`; Moatless's own
    // `interrupted` is the Sandbox going away mid-turn, which is still going to
    // carry on, so T3 keeps hearing `running`.
    let endings: &[(&str, &str, &str, &str)] = &[
        ("t-completed", "taskTurn", "completed", "completed"),
        ("t-agent-completed", "agent", "completed", "completed"),
        ("t-error", "agent", "error", "error"),
        ("t-stopped", "agent", "stopped", "interrupted"),
        ("t-interrupted", "taskTurn", "turn_interrupted", "running"),
    ];

    let viewer = viewer("user-1");
    for (task_id, event_type, sub_type, expected) in endings {
        seed_task(&pool, task_id, "user-1", Some("ws-live")).await;
        let turn = start_turn(&pool, task_id, "run-1").await;

        // Watch from *after* the start, so the only change the feed can carry
        // is the ending itself.
        let mut feed = watching(&pool, &viewer).await;
        ingress(&pool, task_id, turn, event_type, sub_type).await;

        let event = one_row(&pool, &mut feed, &viewer).await;
        assert_eq!(
            event["thread"]["id"], *task_id,
            "{event_type}.{sub_type} delivered someone else's row"
        );
        assert_eq!(
            event["thread"]["latestTurn"]["state"], *expected,
            "{event_type}.{sub_type} should reach the client as `{expected}`"
        );
    }
}

#[sqlx::test]
async fn a_burst_on_one_conversation_delivers_one_row(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    start_turn(&pool, "t-live", "run-1").await;

    let viewer = viewer("user-1");
    let mut feed = watching(&pool, &viewer).await;
    let watched_from = moatless_backend::t3code::conversations_over(&pool)
        .listing_head()
        .await
        .unwrap()
        .position;

    // Four state moves on one conversation between two polls: a new run's turn
    // is allocated, the pod goes away, the run resumes, the turn completes.
    let turn = start_turn(&pool, "t-live", "run-2").await;
    ingress(&pool, "t-live", turn, "taskTurn", "turn_interrupted").await;
    ingress(&pool, "t-live", turn, "taskTurn", "active").await;
    ingress(&pool, "t-live", turn, "agent", "completed").await;

    // The premise: four changes really were recorded. Without this the
    // coalescing assertion below would also pass against a writer that recorded
    // one change in four, which is a different bug and not this one's fix.
    assert_eq!(recorded_after(&pool, watched_from).await, 4);

    let event = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(event["thread"]["id"], "t-live");

    // The row is re-read whole at the burst's last position, so it carries the
    // end state and not the state of whichever change was collapsed last.
    assert_eq!(event["thread"]["latestTurn"]["state"], "completed");
    assert_eq!(
        event["sequence"],
        watched_from + 4,
        "the delivered row stands in for every change up to the burst's last"
    );
}

#[sqlx::test]
async fn a_change_reaches_only_the_clients_whose_listing_holds_it(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-mine", "user-1", Some("ws-live")).await;
    // Not readable: someone else's private conversation.
    seed_task(&pool, "t-theirs", "user-2", Some("ws-live")).await;
    // Readable but not followed — the listing's other filter, and the one a
    // re-read keyed only on access would let through.
    seed_task(&pool, "t-unfollowed", "user-1", Some("ws-live")).await;
    unfollow(&pool, "user-1", "t-unfollowed").await;

    let viewer = viewer("user-1");
    let mut feed = watching(&pool, &viewer).await;
    let watched_from = moatless_backend::t3code::conversations_over(&pool)
        .listing_head()
        .await
        .unwrap()
        .position;

    start_turn(&pool, "t-theirs", "run-1").await;
    start_turn(&pool, "t-unfollowed", "run-1").await;
    start_turn(&pool, "t-mine", "run-1").await;

    // All three turns recorded a change — the feed is filtering, not empty.
    assert_eq!(recorded_after(&pool, watched_from).await, 3);

    let event = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(
        event["thread"]["id"], "t-mine",
        "only what is in this viewer's own listing is delivered"
    );

    // And the cursor moved past the change that was not theirs, rather than
    // stalling on it.
    assert!(delivered(&pool, &mut feed, &viewer).await.is_empty());
}

#[sqlx::test]
async fn a_caught_up_watcher_is_told_nothing(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let mut feed = watching(&pool, &viewer).await;
    let turn = start_turn(&pool, "t-live", "run-1").await;
    one_row(&pool, &mut feed, &viewer).await;

    // Re-delivering the same event changes no row, so nothing is recorded and
    // nothing is sent: the Redis consumer re-applies every event the ingress
    // already committed, and a client that saw one row per re-application would
    // see the sidebar move for work that had already finished.
    ingress(&pool, "t-live", turn, "agent", "completed").await;
    one_row(&pool, &mut feed, &viewer).await;
    ingress(&pool, "t-live", turn, "agent", "completed").await;

    assert!(delivered(&pool, &mut feed, &viewer).await.is_empty());
}

/// A conversation repository that answers the feed's cursor reads and refuses
/// the one that comes after them.
///
/// The point is *where* the refusal lands. `ShellFeed::poll` reads the head,
/// reads the change records, and only then re-reads the rows those records name
/// — and `watch_shell` keeps the subscription alive across a failed poll on the
/// stated grounds that "the cursor has not moved, so the next poll covers the
/// same ground". A cursor advanced before this call makes that sentence false
/// and the batch unrecoverable, with nothing logged on the client's side and
/// nothing wrong on its screen except that it is frozen.
struct RefusesTheRowRead(PgConversationDao);

#[async_trait::async_trait]
impl ConversationDao for RefusesTheRowRead {
    async fn list_followed(&self, viewer: &ScopeContext) -> Result<Vec<Conversation>, sqlx::Error> {
        self.0.list_followed(viewer).await
    }

    async fn list_followed_closed(
        &self,
        viewer: &ScopeContext,
    ) -> Result<Vec<Conversation>, sqlx::Error> {
        self.0.list_followed_closed(viewer).await
    }

    async fn list_followed_by_ids(
        &self,
        _viewer: &ScopeContext,
        _task_ids: &[String],
    ) -> Result<Vec<Conversation>, sqlx::Error> {
        // The shape of the failure this survives: the database is there and the
        // connection is not.
        Err(sqlx::Error::PoolTimedOut)
    }

    // Not the read this double refuses: the shell feed re-reads its rows
    // through the followed listing above, and this resolves one conversation
    // for the thread paths.
    async fn readable_by_id(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<Option<Conversation>, sqlx::Error> {
        self.0.readable_by_id(viewer, task_id).await
    }

    async fn readable_by_ids(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<Vec<Conversation>, sqlx::Error> {
        self.0.readable_by_ids(viewer, task_ids).await
    }

    async fn search_conversations(
        &self,
        viewer: &ScopeContext,
        query: &str,
        limit: i64,
    ) -> Result<Vec<ConversationMatch>, sqlx::Error> {
        self.0.search_conversations(viewer, query, limit).await
    }

    async fn writes_conversation(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<bool, sqlx::Error> {
        self.0.writes_conversation(viewer, task_id).await
    }

    async fn controls_conversation(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<bool, sqlx::Error> {
        self.0.controls_conversation(viewer, task_id).await
    }

    // The thread half of the interface: this stub exists for the shell feed's
    // cursor and is never asked to open a conversation, so both delegate.
    async fn transcript(&self, task_id: &str) -> Result<Transcript, sqlx::Error> {
        self.0.transcript(task_id).await
    }

    async fn source_task_names(
        &self,
        viewer: &ScopeContext,
        task_ids: &[String],
    ) -> Result<HashMap<String, String>, sqlx::Error> {
        self.0.source_task_names(viewer, task_ids).await
    }

    async fn conversation_head(&self, task_id: &str) -> Result<ConversationHead, sqlx::Error> {
        self.0.conversation_head(task_id).await
    }

    async fn conversation_changes(
        &self,
        task_id: &str,
        from: i64,
        limit: i64,
    ) -> Result<Vec<ConversationChange>, sqlx::Error> {
        self.0.conversation_changes(task_id, from, limit).await
    }

    async fn changed_messages(
        &self,
        task_id: &str,
        uuids: &[String],
        activity_ids: &[String],
    ) -> Result<Transcript, sqlx::Error> {
        self.0.changed_messages(task_id, uuids, activity_ids).await
    }

    async fn interactive_call(
        &self,
        task_id: &str,
        tool_use_id: &str,
    ) -> Result<Option<InteractiveCall>, sqlx::Error> {
        self.0.interactive_call(task_id, tool_use_id).await
    }

    async fn listing_head(&self) -> Result<UiRpcListingHead, sqlx::Error> {
        self.0.listing_head().await
    }

    async fn listing_changes_after(
        &self,
        after: i64,
        limit: i64,
    ) -> Result<Vec<UiRpcListingChange>, sqlx::Error> {
        self.0.listing_changes_after(after, limit).await
    }
}

/// A second pool onto the same database, closed. Every query through it fails,
/// which is the workspace half of the same injection with no stub to write.
async fn a_pool_that_has_gone_away(pool: &PgPool) -> PgPool {
    let closed = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    closed.close().await;
    closed
}

/// A poll that fails after reading its change records still delivers them.
///
/// The cursor is the client's only record of what it has been told about, and
/// `watch_shell` deliberately keeps a subscription alive across a transient
/// failure rather than presenting a database blip as a dead sidebar. That
/// decision is sound and its premise is that the failed poll left the cursor
/// alone. Advance the cursor before the reads that build the batch and the two
/// together lose the batch permanently: the next poll starts above it, the
/// client is never told, and it goes on reporting itself live — the silent
/// staleness this whole feed exists to prevent, arriving through its own error
/// handling.
///
/// Both reads that happen after the change records are covered, and both
/// separately, because a cursor moved to sit between them fails only one: a fix
/// that advanced after the conversation re-read but before the workspace read
/// would pass the first half of this test and lose every batch whose workspace
/// read failed.
#[sqlx::test]
async fn a_poll_that_fails_after_reading_its_changes_delivers_them_on_the_next_one(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");
    let (mut feed, mut held) = watcher(&pool, &viewer).await;
    let turn = start_turn(&pool, "t-live", "run-1").await;
    one_row(&pool, &mut feed, &viewer).await;

    // (1) The conversation re-read fails.
    let watched_from = head(&pool).await.position;
    ingress(&pool, "t-live", turn, "agent", "completed").await;
    assert_eq!(
        recorded_after(&pool, watched_from).await,
        1,
        "the premise failed: nothing was recorded for the failed poll to lose"
    );
    let refused = feed
        .poll(
            &moatless_backend::t3code::conversations_over(&pool),
            &RefusesTheRowRead(moatless_backend::t3code::conversations_over(&pool)),
            &defaults(),
            &viewer,
        )
        .await;
    assert!(
        refused.is_err(),
        "the premise failed: the read did not fail"
    );

    let row = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(row["thread"]["id"], "t-live");
    assert_eq!(
        row["thread"]["latestTurn"]["state"], "completed",
        "the turn ending was recorded, read, and then dropped by the poll that \
         failed after moving past it"
    );
    held.apply(&row);

    // (2) The workspace read fails, one statement further on.
    let watched_from = head(&pool).await.position;
    moatless_tasks::task::dao::store_over(&pool)
        .update_task(
            "t-live",
            &TaskPatch {
                name: Some("Renamed".to_string()),
                ..TaskPatch::default()
            },
            &viewer,
        )
        .await
        .unwrap();
    assert_eq!(recorded_after(&pool, watched_from).await, 1);
    let gone = a_pool_that_has_gone_away(&pool).await;
    let refused = feed
        .poll(
            &moatless_backend::t3code::conversations_over(&gone),
            &moatless_backend::t3code::conversations_over(&pool),
            &defaults(),
            &viewer,
        )
        .await;
    assert!(
        refused.is_err(),
        "the premise failed: the read did not fail"
    );

    let item = one_item(&pool, &mut feed, &viewer).await;
    assert_eq!(item["kind"], "thread-upserted");
    assert_eq!(item["thread"]["title"], "Renamed");
    held.apply(&item);
}

#[sqlx::test]
async fn each_delivery_carries_a_position_past_the_one_before_it(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let snapshot = shell_snapshot(
        &moatless_backend::t3code::conversations_over(&pool),
        &moatless_backend::t3code::conversations_over(&pool),
        &defaults(),
        &viewer,
    )
    .await
    .unwrap();
    let mut feed = ShellFeed::resuming_from(snapshot.identity.clone(), snapshot.sequence)
        .holding(snapshot.thread_ids(), snapshot.project_ids());

    let turn = start_turn(&pool, "t-live", "run-1").await;
    let started = one_row(&pool, &mut feed, &viewer).await;
    ingress(&pool, "t-live", turn, "agent", "completed").await;
    let finished = one_row(&pool, &mut feed, &viewer).await;

    // The client discards anything at or below the position it holds, so a
    // delivery that did not advance past the snapshot's would be dropped in
    // silence and the row would never move.
    let started = started["sequence"].as_i64().unwrap();
    let finished = finished["sequence"].as_i64().unwrap();
    assert!(
        started > snapshot.sequence,
        "the start was delivered at {started}, at or below the snapshot's {}",
        snapshot.sequence
    );
    assert!(finished > started);
    assert_eq!(
        finished,
        moatless_backend::t3code::conversations_over(&pool)
            .listing_head()
            .await
            .unwrap()
            .position,
        "the last delivery sits at the position the deployment has reached"
    );
}

// =========================================================================
// Reconnecting
// =========================================================================
//
// A client that comes back holds a position and asks for what it missed. What
// it may be told is one of three things, and the whole of this ticket is that
// they are three rather than two: *here is what changed*, *nothing changed*,
// and *more changed than I kept, here is the list again*. The last two are the
// same empty result unless they are made different, and a client that cannot
// tell them apart renders a stale sidebar forever while reporting itself live.

/// The `snapshot` member of `OrchestrationShellStreamItem`
/// (`orchestration.ts:466-469`). Two keys, both required — the member is what
/// carries "rebuild" in the contract's own vocabulary, so an undeclared key
/// here is a client that decodes nothing and rebuilds never.
const SNAPSHOT_ITEM_REQUIRED: &[&str] = &["kind", "snapshot"];

/// `OrchestrationShellSnapshot` (`orchestration.ts:430-435`).
const SHELL_SNAPSHOT_REQUIRED: &[&str] = &["snapshotSequence", "projects", "threads", "updatedAt"];

/// One rebuild frame, checked against the contract the way a delivered row is.
fn assert_decodable_snapshot_item(item: &Value) {
    assert_keys(item, "snapshot item", SNAPSHOT_ITEM_REQUIRED, &[]);
    assert_eq!(item["kind"], "snapshot");
    assert_keys(
        &item["snapshot"],
        "OrchestrationShellSnapshot",
        SHELL_SNAPSHOT_REQUIRED,
        &[],
    );
    let sequence = item["snapshot"]["snapshotSequence"]
        .as_i64()
        .expect("snapshotSequence must be an integer");
    assert!(sequence >= 0, "`NonNegativeInt` refuses {sequence}");
}

/// Reconnect: what `subscribeShell` hands a client that arrives holding
/// `after`, and the feed it carries on with.
async fn reconnect(
    pool: &PgPool,
    viewer: &ScopeContext,
    after: Option<i64>,
) -> (Vec<Value>, ShellFeed) {
    let environment = current_head(pool).await.unwrap().identity;
    reconnect_to(pool, &environment, viewer, after).await
}

/// The same reconnection, for a deployment that started against `environment`
/// and is now reading `pool`.
///
/// The two are the same database in every test but the restore ones, which is
/// the whole of what those tests vary: the process is unchanged, its clients are
/// unchanged, and the database underneath it is not the one it started with.
async fn reconnect_to(
    pool: &PgPool,
    environment: &UiRpcEnvironmentIdentity,
    viewer: &ScopeContext,
    after: Option<i64>,
) -> (Vec<Value>, ShellFeed) {
    let subscription = shell_subscription(
        &moatless_backend::t3code::conversations_over(pool),
        &moatless_backend::t3code::conversations_over(pool),
        &defaults(),
        viewer,
        environment,
        after,
    )
    .await
    .unwrap();
    (subscription.values, subscription.feed)
}

/// Advance the deployment's position past the resume window without pretending
/// the changes belong to anything.
///
/// They are recorded through the real allocator, on a Task id nothing in the
/// listing references, so the head moves exactly as it would after a few days
/// of other people's work — which is the disconnection acceptance criterion 2
/// is about.
async fn other_peoples_work(pool: &PgPool, count: i64) {
    let mut tx = pool.begin().await.unwrap();
    for index in 0..count {
        record_change(&mut tx, &format!("t-elsewhere-{index}"))
            .await
            .unwrap();
    }
    tx.commit().await.unwrap();
}

/// Acceptance criterion 1. Away briefly, things happened, and the list the
/// client already had is updated rather than replaced.
///
/// The teeth are the negative half: `assert!(!rebuilt)`. Before this ticket
/// every subscription answered with a snapshot, so an implementation that
/// ignores `afterSequence` fails here — and so does one that rebuilds whenever
/// it is unsure.
#[sqlx::test]
async fn a_client_that_missed_changes_is_replayed_rather_than_rebuilt(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");

    // It read the listing, then the connection dropped.
    let held = moatless_backend::t3code::conversations_over(&pool)
        .listing_head()
        .await
        .unwrap()
        .position;

    // While it was away, a turn ran to completion on a conversation it follows.
    let turn = start_turn(&pool, "t-live", "run-1").await;
    ingress(&pool, "t-live", turn, "agent", "completed").await;
    assert!(
        recorded_after(&pool, held).await >= 2,
        "the premise failed: nothing was recorded while the client was away"
    );

    let (values, _feed) = reconnect(&pool, &viewer, Some(held)).await;

    assert!(
        !values.iter().any(|item| item["kind"] == "snapshot"),
        "the client was handed a rebuilt listing instead of what it missed: {values:#?}"
    );
    assert_eq!(
        values.len(),
        1,
        "the burst on one conversation coalesces into one row, got {values:#?}"
    );
    assert_decodable_upsert(&values[0]);
    assert_eq!(values[0]["thread"]["id"], "t-live");
    assert_eq!(
        values[0]["thread"]["latestTurn"]["state"], "completed",
        "the row carries the state the client missed, not the state it left"
    );
    assert!(values[0]["sequence"].as_i64().unwrap() > held);
}

/// Acceptance criterion 4, with the teeth the criterion does not have on its
/// own.
///
/// "Replaying from the position a client holds returns nothing" passes against
/// an implementation that always returns nothing — which is exactly the dead
/// path this ticket replaces, so on its own it is worth nothing. So the test is
/// asymmetric, and every half is a break somebody could ship:
///
/// - A change is recorded *before* the position the client holds, so there is
///   something to wrongly re-send. Replaying `WHERE position >= cursor` instead
///   of `>` fails here, which is the second clause of the criterion.
/// - A second change is recorded, and **replaying from the same held position
///   again must now carry it**. An always-empty replay fails here.
/// - The feed that same subscription was handed must deliver it live too, so
///   the empty first answer was a cursor in the right place rather than a
///   subscription that never started.
///
/// Verified by making each of those three breaks and watching this fail.
#[sqlx::test]
async fn a_caught_up_client_is_replayed_nothing_and_still_hears_what_comes_next(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");

    // A change happens, and the client's position is taken *after* it — so this
    // is the change that produced the position it holds.
    let turn = start_turn(&pool, "t-live", "run-1").await;
    let held = moatless_backend::t3code::conversations_over(&pool)
        .listing_head()
        .await
        .unwrap()
        .position;
    assert!(held > 0, "the premise failed: nothing produced a position");
    assert_eq!(
        recorded_after(&pool, held).await,
        0,
        "the premise failed: the client is not actually caught up"
    );

    let (values, mut feed) = reconnect(&pool, &viewer, Some(held)).await;
    assert!(
        values.is_empty(),
        "a caught-up client was told something; the change that produced its \
         own position was most likely re-sent: {values:#?}"
    );

    ingress(&pool, "t-live", turn, "agent", "completed").await;

    // Replaying from the *same* position now carries something. Without this,
    // the assertion above is satisfied by a replay that can never answer at
    // all — which is the path this ticket was opened to replace.
    let (now, _) = reconnect(&pool, &viewer, Some(held)).await;
    assert_eq!(
        now.len(),
        1,
        "replaying from a position with a change past it returned nothing: {now:#?}"
    );
    assert_decodable_upsert(&now[0]);

    // And the feed the first reconnection was handed is live: the same change
    // reaches it without a second subscription.
    let event = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(event["thread"]["id"], "t-live");
    assert!(event["sequence"].as_i64().unwrap() > held);
}

/// Acceptance criteria 2 and 3 together, because they are the same assertion
/// from two sides: the answer to "too much happened" is a listing, and it is
/// not the answer to "nothing happened".
///
/// Both clients below are told something the other is not. Collapse the two
/// into one answer — return `[]` for the gap, or a snapshot for both — and one
/// of these halves fails.
#[sqlx::test]
async fn a_gap_too_large_to_replay_is_a_fresh_listing_and_silence_is_not(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");

    // Someone who has been away a long time: their position, then more changes
    // than the resume window keeps.
    let stale = moatless_backend::t3code::conversations_over(&pool)
        .listing_head()
        .await
        .unwrap()
        .position;
    other_peoples_work(&pool, RESUME_WINDOW + 1).await;
    let head = moatless_backend::t3code::conversations_over(&pool)
        .listing_head()
        .await
        .unwrap()
        .position;
    assert_eq!(head - stale, RESUME_WINDOW + 1, "the premise failed");

    let (gap, _feed) = reconnect(&pool, &viewer, Some(stale)).await;
    assert_eq!(
        gap.len(),
        1,
        "a client past the resume window should be handed one listing: {gap:#?}"
    );
    assert_decodable_snapshot_item(&gap[0]);
    assert_eq!(
        gap[0]["snapshot"]["snapshotSequence"].as_i64().unwrap(),
        head,
        "the rebuilt listing resumes from where the deployment actually is"
    );
    // Nothing is quietly absent from it: the conversation the client follows is
    // in the list it was handed, not merely implied by an event it never got.
    let threads = gap[0]["snapshot"]["threads"].as_array().unwrap();
    assert_eq!(threads.len(), 1, "the rebuilt listing is missing rows");
    assert_eq!(threads[0]["id"], "t-live");

    // And the other side of the distinction, on the same deployment: a client
    // that is up to date is told nothing at all, which is a different frame and
    // not a smaller one.
    let (silence, _feed) = reconnect(&pool, &viewer, Some(head)).await;
    assert!(
        silence.is_empty(),
        "\"nothing happened\" and \"too much happened\" are the same answer: {silence:#?}"
    );
}

/// A cursor one past the window is rebuilt and a cursor exactly on it is
/// replayed, so the boundary is where the constant says it is rather than
/// somewhere near it.
#[sqlx::test]
async fn the_resume_window_is_exclusive_at_exactly_its_own_width(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");

    other_peoples_work(&pool, RESUME_WINDOW + 1).await;
    let head = moatless_backend::t3code::conversations_over(&pool)
        .listing_head()
        .await
        .unwrap()
        .position;

    let (edge, _feed) = reconnect(&pool, &viewer, Some(head - RESUME_WINDOW)).await;
    assert!(
        !edge.iter().any(|item| item["kind"] == "snapshot"),
        "a cursor exactly at the window's width was rebuilt: {edge:#?}"
    );

    let (past, _feed) = reconnect(&pool, &viewer, Some(head - RESUME_WINDOW - 1)).await;
    assert_eq!(past.len(), 1);
    assert_eq!(past[0]["kind"], "snapshot");
}

/// The spec's Decision 7, first guard: a position this deployment has never
/// reached is impossible, and is answered with a listing rather than an empty
/// reply.
///
/// This is the failure nothing else watches. T3 guards against the client
/// falling behind the server; neither side guards against the server falling
/// behind the client. A client asking for everything after a position that will
/// never be allocated receives nothing, forever, while reporting itself live.
///
/// (The other guard — an identity derived from the database, which catches a
/// *reused* position after a restore — is ticket 07's.)
#[sqlx::test]
async fn a_position_the_deployment_has_never_reached_is_rebuilt_not_answered_with_silence(
    pool: PgPool,
) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");

    let head = moatless_backend::t3code::conversations_over(&pool)
        .listing_head()
        .await
        .unwrap()
        .position;

    for impossible in [head + 1, head + 1_000_000] {
        let (values, _feed) = reconnect(&pool, &viewer, Some(impossible)).await;
        assert_eq!(
            values.len(),
            1,
            "a client resuming from {impossible}, which this deployment has \
             never reached, was not given a listing: {values:#?}"
        );
        assert_decodable_snapshot_item(&values[0]);
    }
}

/// A client holding nothing gets the listing, exactly as before this ticket.
///
/// Kept because the resume path is the same function: a change to how
/// `afterSequence` is read must not turn the first connection of a fresh client
/// into an empty sidebar.
#[sqlx::test]
async fn a_client_holding_no_position_is_sent_the_listing(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");

    let (values, _feed) = reconnect(&pool, &viewer, None).await;
    assert_eq!(values.len(), 1);
    assert_decodable_snapshot_item(&values[0]);
    assert_eq!(
        values[0]["snapshot"]["threads"].as_array().unwrap().len(),
        1
    );
}

/// A replay is filtered by the same two rules the snapshot is, because it is
/// the same query — someone else's change advances the cursor and tells this
/// viewer nothing.
///
/// Worth its own test because replay is the one path that could plausibly be
/// written from the change records themselves, which carry no viewer. That
/// version leaks the id of every conversation in the deployment to everyone.
#[sqlx::test]
async fn a_replay_does_not_carry_changes_the_viewer_may_not_see(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    // Someone else's conversation, in the same project, which `user-1` does not
    // follow. `insert_task` follows for its creator, so only `user-2` has it.
    seed_task(&pool, "t-theirs", "user-2", Some("ws-live")).await;
    let viewer = viewer("user-1");

    let held = moatless_backend::t3code::conversations_over(&pool)
        .listing_head()
        .await
        .unwrap()
        .position;
    start_turn(&pool, "t-theirs", "run-theirs").await;
    assert_eq!(
        recorded_after(&pool, held).await,
        1,
        "the premise failed: the change the viewer must not see was not recorded"
    );

    let (values, mut feed) = reconnect(&pool, &viewer, Some(held)).await;
    assert!(
        values.is_empty(),
        "a replay carried someone else's conversation: {values:#?}"
    );

    // And the cursor moved over it regardless, so this viewer's feed is not
    // stalled behind work that will never be theirs.
    start_turn(&pool, "t-live", "run-mine").await;
    let event = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(event["thread"]["id"], "t-live");
}

// =========================================================================
// The database changing underneath a deployment
// =========================================================================
//
// The spec's Decision 7 is two guards, and the whole of it is that neither is
// redundant. A position higher than any this deployment has reached is
// impossible; a database that is not the one this process started against makes
// every position it ever handed out mean something else. Each has a case the
// other cannot see, and the two tests below are those two cases — written so
// that deleting either guard turns exactly one of them red.
//
// Both are operational disasters, so both are built rather than described. The
// counter is really reset; the restored database is really a second database,
// created in the cluster and migrated, with positions really reissued to
// different changes. A test that claims to simulate a restore and does not is
// worse than no test, because it will be cited.

/// A backup, restored into a new database beside the original.
///
/// `CREATE DATABASE ... TEMPLATE` is a byte-for-byte copy, and that is what
/// makes this a restore rather than a database that merely looks like one:
/// **every row the origin holds at this moment is in the copy.** Anything a
/// design were tempted to keep the environment's identity in — a UUID seeded by
/// a migration, a row written at first boot — is copied along with the rest and
/// reports the copy as the original. A restore is not distinguishable from
/// inside the data, which is why [`UiRpcEnvironmentIdentity`] is not in it.
///
/// The copy is made **in the same cluster**, so the cluster's system identifier
/// is equal on both sides too. Everything these tests catch is therefore caught
/// by the database's OID alone, which is the half of the identity easiest to
/// argue away as decoration.
///
/// Postgres refuses to copy a template anything else is connected to, so the
/// deployment's own pool is closed for the moment the copy takes and reopened
/// as [`RestoredDatabase::origin`]. That is also what a restore looks like.
struct RestoredDatabase {
    /// The database the deployment started against, reopened after the copy.
    origin: PgPool,
    /// The restore, which the deployment is then pointed at.
    pool: PgPool,
    name: String,
    cluster: PgPool,
}

impl RestoredDatabase {
    async fn discard(self) {
        let name = self.name;
        self.origin.close().await;
        self.pool.close().await;
        sqlx::query(AssertSqlSafe(format!(
            r#"DROP DATABASE IF EXISTS "{name}""#
        )))
        .execute(&self.cluster)
        .await
        .unwrap();
        self.cluster.close().await;
    }
}

async fn back_up_and_restore(pool: PgPool) -> RestoredDatabase {
    let options = pool.connect_options().as_ref().clone();
    let current: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .unwrap();
    // Postgres truncates an identifier at 63 bytes and `#[sqlx::test]` names its
    // databases right up to that limit, so a plain suffix would truncate back to
    // the name it was derived from — and the `DROP` below would be the test
    // dropping the database it is restoring.
    let name = format!("{}_restored", &current[..current.len().min(53)]);
    assert_ne!(name, current);

    pool.close().await;
    let cluster = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.clone().database("postgres"))
        .await
        .unwrap();
    for statement in [
        format!(r#"DROP DATABASE IF EXISTS "{name}""#),
        format!(r#"CREATE DATABASE "{name}" TEMPLATE "{current}""#),
    ] {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&cluster)
            .await
            .unwrap();
    }

    let reopen = |options| async move {
        PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .unwrap()
    };
    RestoredDatabase {
        origin: reopen(options.clone()).await,
        pool: reopen(options.database(&name)).await,
        name,
        cluster,
    }
}

/// The cluster and the database a pool is connected to, read straight from
/// Postgres rather than through [`UiRpcEnvironmentIdentity`], so a test can say which
/// half of the identity it is exercising.
async fn cluster_and_oid(pool: &PgPool) -> (String, i64) {
    sqlx::query_as(
        "SELECT (SELECT system_identifier::text FROM pg_control_system()),
                (SELECT oid::int8 FROM pg_database WHERE datname = current_database())",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn head(pool: &PgPool) -> UiRpcListingHead {
    current_head(pool).await.unwrap()
}

/// The identity is a property of the database and of nothing else: it does not
/// move between reads or between connections, and a restored copy does not
/// share it.
///
/// The first half is what stops the guard firing on its own deployment — an
/// identity that varied by connection would rebuild every client every poll.
/// The second half is the half the guard is for, and it is asserted against a
/// copy that is **identical in every row** and sits in the **same cluster**, so
/// neither the data nor the system identifier is carrying it. The database's
/// OID is the only thing left, and it is enough.
///
/// # The privilege this needs, asserted rather than assumed
///
/// `the_identity_is_readable_by_a_least_privilege_application_role` below is the
/// companion this one cannot be: every `#[sqlx::test]` connects as the role that
/// created the database, which on any ordinary harness is a superuser. A
/// statement that silently required superuser would pass every test in this file
/// and fail at startup on a least-privilege deployment.
#[sqlx::test]
async fn the_environment_identity_is_stable_and_names_one_database(pool: PgPool) {
    // Something in the tables, so "identical in every row" is a claim with
    // content rather than two empty schemas agreeing.
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    other_peoples_work(&pool, 20).await;

    let restored = back_up_and_restore(pool).await;
    let origin = &restored.origin;

    assert_eq!(head(origin).await.identity, head(origin).await.identity);

    // A second pool, so a second connection, on the same database.
    let second = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(origin.connect_options().as_ref().clone())
        .await
        .unwrap();
    assert_eq!(
        head(origin).await.identity,
        head(&second).await.identity,
        "the identity moved between two connections to the same database"
    );
    second.close().await;

    // The copy really is a copy: same counter, same records, same rows.
    assert_eq!(
        head(&restored.pool).await.position,
        head(origin).await.position
    );
    assert_eq!(
        thread_ids(&restored.pool, &viewer("user-1")).await,
        thread_ids(origin, &viewer("user-1")).await
    );

    let (origin_cluster, origin_oid) = cluster_and_oid(origin).await;
    let (restored_cluster, restored_oid) = cluster_and_oid(&restored.pool).await;
    assert_eq!(
        origin_cluster, restored_cluster,
        "the premise failed: the restore is meant to be in the same cluster"
    );
    assert_ne!(origin_oid, restored_oid);

    // Both halves, pinned against the values Postgres reports for them.
    //
    // The OID half is what the restore below exercises. The cluster half
    // cannot be — a second cluster is not something `#[sqlx::test]` can
    // produce — and it is the half most easily mistaken for decoration, so it
    // is pinned structurally instead. Dropping it would leave the identity
    // resting on a per-cluster counter that starts at 16,384 in every cluster
    // alive, and a restore into a freshly-`initdb`'d server would present as
    // the database it replaced.
    assert_eq!(
        head(origin).await.identity,
        UiRpcEnvironmentIdentity::new(format!("{origin_cluster}:{origin_oid}")),
        "the identity is no longer the cluster's system identifier and the \
         database's OID"
    );

    assert_ne!(
        head(origin).await.identity,
        head(&restored.pool).await.identity,
        "a restored copy reported the same identity as its original; every \
         position the original ever issued now looks valid in the copy"
    );

    restored.discard().await;
}

/// The identity statement runs as an ordinary application role.
///
/// `current_head` reads the cluster's system identifier through
/// `pg_control_system()`, and `router_from_env` returns `Err` rather than
/// falling back to a constant when that read fails — deliberately, because a
/// constant identity is an identity that never changes when the database does.
/// The consequence is that a `pg_control_system()` an ordinary role could not
/// execute would take the T3 UI RPC router down at startup on exactly the
/// deployments that are configured correctly, and no other test in this suite
/// could see it: `#[sqlx::test]` connects as the role that created the
/// database, which is a superuser on every harness this runs on.
///
/// So a role is made here that has **one** privilege in this database — `SELECT`
/// on the counter table — and the statement is run as it. Anything else the
/// statement comes to need shows up here as a `permission denied`, naming the
/// object.
///
/// The empirical answer, for the record: `pg_control_system()` is never
/// `REVOKE`d at `initdb`. Its `pg_proc.proacl` is null on PostgreSQL 13, 16 and
/// 17, which is the default — `EXECUTE` to `PUBLIC`. The functions that *are*
/// restricted (`pg_ls_logdir`, `pg_read_file`, `pg_stat_file`, `pg_ls_dir`,
/// `pg_switch_wal`) carry an explicit `{postgres=X/postgres}` and a matching
/// `REVOKE EXECUTE ... FROM public` in the image's `system_functions.sql`;
/// `pg_control_system` appears in neither file.
#[sqlx::test]
async fn the_identity_is_readable_by_a_least_privilege_application_role(pool: PgPool) {
    // Something for the counter to have counted, so the position read is not
    // trivially satisfied by an empty table.
    seed_live_project(&pool).await;

    let options = pool.connect_options().as_ref().clone();
    let database: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&pool)
        .await
        .unwrap();
    // Cluster-wide and therefore shared with every other test running against
    // this server, so the name is derived from this test's own database.
    let role = format!("{}_least", &database[..database.len().min(50)]);

    for statement in [
        format!(r#"DROP ROLE IF EXISTS "{role}""#),
        format!(
            r#"CREATE ROLE "{role}" LOGIN PASSWORD 'least' NOSUPERUSER NOCREATEDB NOCREATEROLE"#
        ),
        // The whole grant. Not `pg_read_all_stats`, not `pg_monitor`, not the
        // database owner: if the identity read needed any of those, this is
        // where it would say so.
        format!(r#"GRANT SELECT ON task_listing_position TO "{role}""#),
    ] {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&pool)
            .await
            .unwrap();
    }

    let least = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options.username(&role).password("least"))
        .await
        .unwrap();
    let ordinary = current_head(&least).await;
    least.close().await;

    // Cleaned up before the assertion, so a failure does not leave a role
    // behind for the next run to trip over.
    for statement in [
        format!(r#"DROP OWNED BY "{role}""#),
        format!(r#"DROP ROLE IF EXISTS "{role}""#),
    ] {
        sqlx::query(AssertSqlSafe(statement))
            .execute(&pool)
            .await
            .unwrap();
    }

    let ordinary = ordinary.unwrap_or_else(|error| {
        panic!(
            "an ordinary application role cannot read the environment identity, \
             so `router_from_env` fails to start on a least-privilege \
             deployment: {error}"
        )
    });
    assert_eq!(
        ordinary,
        head(&pool).await,
        "the identity a least-privilege role reads is not the one the \
         deployment believes it is serving"
    );
}

/// **Acceptance criterion 4, and the case only the position guard sees.**
///
/// The counter is reset to zero on a database that is otherwise untouched. Its
/// cluster identifier and its OID are exactly what they were — asserted below,
/// because that assertion is the point: the identity guard's inputs are
/// identical to a healthy deployment's, so it provably contributes nothing here
/// and the position guard is carrying this alone.
///
/// Verified by deleting the `after > head` comparison from `task_listing::resume`
/// and watching this fail: the client is admitted at the position it holds, the
/// replay reads an emptied table, and it is handed `[]` — told it is up to date
/// while the deployment it is watching has started counting again from one.
#[sqlx::test]
async fn a_counter_reset_on_the_same_database_is_caught_by_the_position_guard(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");
    other_peoples_work(&pool, 200).await;

    let before = head(&pool).await;
    let held = before.position;
    assert!(held > 0, "the premise failed: nothing produced a position");

    // The counter goes back to zero, and the records it issued go with it —
    // which is what a reset has to look like, since `task_listing_changes`
    // keys on the position and a counter starting again would collide.
    sqlx::query("DELETE FROM task_listing_position")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("TRUNCATE task_listing_changes")
        .execute(&pool)
        .await
        .unwrap();

    let after = head(&pool).await;
    assert_eq!(
        after.position, 0,
        "the premise failed: the counter did not reset"
    );
    assert_eq!(
        after.identity, before.identity,
        "the premise failed: resetting the counter changed the database's \
         identity, so this is not a case only the position guard can see"
    );

    let (values, mut feed) = reconnect_to(&pool, &before.identity, &viewer, Some(held)).await;
    assert_eq!(
        values.len(),
        1,
        "a client holding {held} on a deployment that has reset to zero was not \
         given a listing: {values:#?}"
    );
    assert_decodable_snapshot_item(&values[0]);
    assert_eq!(values[0]["snapshot"]["snapshotSequence"], 0);

    // And it is watching again rather than parked above a counter that will
    // take 200 changes to reach it: the next thing that happens arrives.
    start_turn(&pool, "t-live", "run-1").await;
    let event = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(event["thread"]["id"], "t-live");
    assert_eq!(event["sequence"], 1);
}

/// **Acceptance criteria 2 and 3, and the case only the identity guard sees.**
///
/// The sequence is the spec's own, built rather than described:
///
/// 1. The deployment runs, and a backup is taken at position 100.
/// 2. It runs on to position 200. A client reads the listing there and holds
///    200 — a listing that includes a conversation started after the backup.
/// 3. The database is lost and the backup is restored beside it. The copy is
///    back at 100, and it counts forward again: positions 101 through 200 are
///    **handed out a second time, to entirely different changes**.
/// 4. The client comes back holding 200.
///
/// Every number now looks right, which is the whole difficulty — the assertion
/// below proves it by asking `resume` what it would say if the two identities
/// matched, and it says replay. So without the identity guard the client is
/// told what changed *after* 200 and folds it into the listing it already had:
/// it keeps a conversation the restore has never heard of, and never hears
/// about the hundred changes that were reissued underneath it. That is
/// criterion 3's "silently skipping the changes in between", exactly.
///
/// Verified by deleting the identity comparison from `task_listing::resume` and
/// watching this fail: `values` came back as one `thread-upserted` for
/// `t-after-restore`, and `assert_decodable_snapshot_item` reported `kind` as
/// `thread-upserted` where the contract's rebuild member was required.
#[sqlx::test]
async fn a_restored_backup_reissuing_positions_is_caught_by_the_identity_guard(pool: PgPool) {
    let viewer = viewer("user-1");

    // 1. The deployment, and the backup.
    seed_live_project(&pool).await;
    seed_task(&pool, "t-before-backup", "user-1", Some("ws-live")).await;
    other_peoples_work(&pool, 100).await;
    let restored = back_up_and_restore(pool).await;
    let backed_up_at = head(&restored.pool).await.position;

    // 2. The original runs on, and the client reads the listing there.
    seed_task(
        &restored.origin,
        "t-after-backup",
        "user-1",
        Some("ws-live"),
    )
    .await;
    other_peoples_work(&restored.origin, 100).await;
    let original = head(&restored.origin).await;
    let held = original.position;
    assert!(
        held > backed_up_at,
        "the premise failed: the backup is not behind"
    );
    assert!(
        thread_ids(&restored.origin, &viewer)
            .await
            .contains(&"t-after-backup".to_string()),
        "the premise failed: the client's listing does not hold anything the \
         backup is missing"
    );

    // 3. The restore counts forward again, reissuing 101..=200 to other work.
    seed_task(&restored.pool, "t-after-restore", "user-1", Some("ws-live")).await;
    let reissued = held - head(&restored.pool).await.position;
    other_peoples_work(&restored.pool, reissued).await;
    start_turn(&restored.pool, "t-after-restore", "run-1").await;
    let now = head(&restored.pool).await;
    assert!(
        reissued > 0 && now.position > held,
        "the premise failed: the restore did not reissue the client's positions"
    );

    // 4. Every other guard admits this cursor, so anything that refuses it is
    //    the identity.
    assert_eq!(
        resume(&now.identity, &now.identity, now.position, held),
        UiRpcResume::From(held),
        "the premise failed: a guard other than the identity already refuses \
         this cursor, so this test is not about the identity guard"
    );

    let (values, _feed) =
        reconnect_to(&restored.pool, &original.identity, &viewer, Some(held)).await;
    assert_eq!(
        values.len(),
        1,
        "a client resuming into a restored database was replayed from a position \
         that now belongs to different changes: {values:#?}"
    );
    assert_eq!(
        values[0]["kind"],
        "snapshot",
        "the client was replayed a difference against a listing it built in a \
         database that no longer exists; positions {} to {held} were handed out \
         a second time and it will never hear about them: {values:#?}",
        backed_up_at + 1
    );
    assert_decodable_snapshot_item(&values[0]);

    // The listing it is given is the restore's, whole. The client replaces what
    // it holds with this, so the conversation that only ever existed after the
    // backup goes rather than lingering forever in a sidebar nothing updates.
    let mut threads: Vec<&str> = values[0]["snapshot"]["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|thread| thread["id"].as_str().unwrap())
        .collect();
    threads.sort_unstable();
    assert_eq!(threads, vec!["t-after-restore", "t-before-backup"]);
    assert_eq!(values[0]["snapshot"]["snapshotSequence"], now.position);

    restored.discard().await;
}

/// **Acceptance criterion 2, for the client that never reconnects.**
///
/// A subscription that was open when the database was replaced is the one path
/// no resume guard covers, because it does not resume: the socket is fine, the
/// cursor is where it was, and every poll asks for everything past a position
/// the restore has not reached yet. It is answered with silence, and then —
/// once the restore counts past that position — with changes, having skipped
/// everything in between.
///
/// Verified by deleting the identity check from `ShellFeed::poll` and watching
/// this fail on the first assertion: the poll returned `[]`, which the
/// subscription reads as "nothing changed" and sends nowhere. The sidebar is
/// then frozen for as long as the connection lasts, while the client renders
/// itself live — which is the sentence this ticket is named after.
#[sqlx::test]
async fn a_connected_client_whose_database_is_replaced_rebuilds_rather_than_stalling(pool: PgPool) {
    let viewer = viewer("user-1");
    seed_live_project(&pool).await;
    seed_task(&pool, "t-before-backup", "user-1", Some("ws-live")).await;
    other_peoples_work(&pool, 100).await;

    let restored = back_up_and_restore(pool).await;
    other_peoples_work(&restored.origin, 100).await;

    // Watching, and up to date, on the database as it was.
    let mut feed = watching(&restored.origin, &viewer).await;
    let held = head(&restored.origin).await.position;
    assert!(
        delivered(&restored.origin, &mut feed, &viewer)
            .await
            .is_empty()
    );

    // The deployment is repointed at the restore, which is behind the cursor.
    seed_task(&restored.pool, "t-after-restore", "user-1", Some("ws-live")).await;
    assert!(
        head(&restored.pool).await.position < held,
        "the premise failed: the restore has already passed the cursor, so a \
         stall would not be observable"
    );

    let items = delivered(&restored.pool, &mut feed, &viewer).await;
    assert_eq!(
        items.len(),
        1,
        "a subscription whose database was replaced was told nothing: {items:#?}"
    );
    assert_eq!(
        items[0]["kind"], "snapshot",
        "a subscription whose database was replaced was sent a difference \
         against the listing it holds: {items:#?}"
    );
    assert_decodable_snapshot_item(&items[0]);
    let mut threads: Vec<&str> = items[0]["snapshot"]["threads"]
        .as_array()
        .unwrap()
        .iter()
        .map(|thread| thread["id"].as_str().unwrap())
        .collect();
    threads.sort_unstable();
    assert_eq!(threads, vec!["t-after-restore", "t-before-backup"]);

    // The same feed then carries on in the restore rather than rebuilding every
    // poll: it took the new identity and the new position with it.
    assert!(
        delivered(&restored.pool, &mut feed, &viewer)
            .await
            .is_empty()
    );
    start_turn(&restored.pool, "t-after-restore", "run-1").await;
    let event = one_row(&restored.pool, &mut feed, &viewer).await;
    assert_eq!(event["thread"]["id"], "t-after-restore");

    restored.discard().await;
}

/// `ExecutionEnvironmentDescriptor` (`environment.ts:57-63`). Every key
/// required; the client decodes this before it has a connection, so an
/// undeclared one is an environment nothing can reach.
const DESCRIPTOR_REQUIRED: &[&str] = &[
    "environmentId",
    "label",
    "platform",
    "serverVersion",
    "capabilities",
];

/// The public descriptor, as a client reads it before any login.
async fn descriptor(pool: &PgPool) -> Value {
    std::env::set_var("T3_UI_RPC_ENABLED", "true");
    std::env::set_var("T3_UI_RPC_ALLOWED_ORIGINS", "http://localhost:5733");
    let router = router_from_env(mounted_dependencies(pool))
        .await
        .unwrap()
        .expect("the compatibility routes are gated on")
        .router;
    let response = router
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/.well-known/t3/environment")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    body_json(response).await
}

/// The identity half of Decision 7, on the wire.
///
/// T3 keys everything it caches for an environment on `environmentId`
/// (`client-runtime/src/platform/persistence.ts:59-63`), so this string is what
/// makes a client with a stale listing discard it without being told to. A
/// deployment that reported a configured constant here would present a replaced
/// database as the same environment, and the cached listing would survive the
/// thing it describes.
#[sqlx::test]
async fn the_environment_descriptor_reports_the_database_it_is_serving(pool: PgPool) {
    let published = descriptor(&pool).await;
    assert_keys(
        &published,
        "ExecutionEnvironmentDescriptor",
        DESCRIPTOR_REQUIRED,
        &[],
    );
    let reported = published["environmentId"].as_str().unwrap().to_string();
    assert_ne!(
        reported, "moatless",
        "the environment id is still the configured constant, so no database \
         change can move it"
    );
    assert!(
        reported.starts_with("moatless-"),
        "the configured name is no longer the readable half of {reported}"
    );
    assert_eq!(
        descriptor(&pool).await["environmentId"],
        reported,
        "the environment id moved without the database moving; every client \
         would discard its cached listing on every restart"
    );

    let restored = back_up_and_restore(pool).await;
    assert_eq!(
        descriptor(&restored.origin).await["environmentId"],
        Value::String(reported.clone()),
        "the environment id moved when nothing about the database did"
    );
    assert_ne!(
        descriptor(&restored.pool).await["environmentId"],
        Value::String(reported),
        "a restored copy of this database presents as the same environment, so \
         a client keeps the listing it built against the original"
    );
    restored.discard().await;
}

// =========================================================================
// The rest of the change kinds, delivered
// =========================================================================
//
// Ticket 04 delivered a turn moving. What follows is everything else a sidebar
// shows moving, and the two directions ticket 04 could not express: a row
// *entering* a listing, and a row *leaving* one.

/// The other three members of `OrchestrationShellStreamEvent`
/// (`orchestration.ts:438-459`), and the two non-event members of
/// `OrchestrationShellStreamItem` (`:462-471`).
const THREAD_REMOVED_REQUIRED: &[&str] = &["kind", "sequence", "threadId"];
const PROJECT_UPSERTED_REQUIRED: &[&str] = &["kind", "sequence", "project"];
const PROJECT_REMOVED_REQUIRED: &[&str] = &["kind", "sequence", "projectId"];

/// `OrchestrationShellSnapshot` (`:430-435`) and `OrchestrationProjectShell`
/// (`:389-398`).
const SNAPSHOT_REQUIRED: &[&str] = &["snapshotSequence", "projects", "threads", "updatedAt"];
const PROJECT_REQUIRED: &[&str] = &[
    "id",
    "title",
    "workspaceRoot",
    "defaultModelSelection",
    "scripts",
    "createdAt",
    "updatedAt",
];
const PROJECT_OPTIONAL: &[&str] = &["repositoryIdentity", "scriptsEditable"];
const PROJECT_NON_EMPTY: &[&str] = &["id", "title", "workspaceRoot"];

/// One item off the feed, checked against the whole union rather than the one
/// member a test happens to expect. An item the client cannot decode is not a
/// missing update — the stream's decode fails and the subscription stops.
fn assert_decodable_item(item: &Value) {
    let sequence = || {
        let sequence = item["sequence"]
            .as_i64()
            .unwrap_or_else(|| panic!("{} .sequence must be an integer", item["kind"]));
        assert!(sequence >= 0, "`NonNegativeInt` refuses {sequence}");
    };
    match item["kind"].as_str() {
        Some("thread-upserted") => assert_decodable_upsert(item),
        Some("thread-removed") => {
            assert_keys(item, "thread-removed", THREAD_REMOVED_REQUIRED, &[]);
            sequence();
            assert_non_empty(item, "thread-removed", &["threadId"]);
        }
        Some("project-upserted") => {
            assert_keys(item, "project-upserted", PROJECT_UPSERTED_REQUIRED, &[]);
            sequence();
            assert_keys(
                &item["project"],
                "project",
                PROJECT_REQUIRED,
                PROJECT_OPTIONAL,
            );
            assert_non_empty(&item["project"], "project", PROJECT_NON_EMPTY);
        }
        Some("project-removed") => {
            assert_keys(item, "project-removed", PROJECT_REMOVED_REQUIRED, &[]);
            sequence();
            assert_non_empty(item, "project-removed", &["projectId"]);
        }
        Some("snapshot") => {
            assert_keys(item, "snapshot", SNAPSHOT_ITEM_REQUIRED, &[]);
            assert_keys(&item["snapshot"], "shell snapshot", SNAPSHOT_REQUIRED, &[]);
            for thread in item["snapshot"]["threads"].as_array().unwrap() {
                assert_decodable_thread(thread);
            }
            for project in item["snapshot"]["projects"].as_array().unwrap() {
                assert_keys(project, "project", PROJECT_REQUIRED, PROJECT_OPTIONAL);
            }
        }
        other => panic!("`{other:?}` is not a member of `OrchestrationShellStreamItem`"),
    }
}

/// Exactly one item, decoded. The counterpart of `one_row` for the deliveries
/// that are not conversation rows.
async fn one_item(pool: &PgPool, feed: &mut ShellFeed, viewer: &ScopeContext) -> Value {
    let items = delivered(pool, feed, viewer).await;
    assert_eq!(items.len(), 1, "expected exactly one item, got {items:#?}");
    assert_decodable_item(&items[0]);
    items[0].clone()
}

/// The ids a client holds after applying everything it has been sent, folded
/// exactly as `applyShellStreamEvent` folds them
/// (`client-runtime/src/state/shellReducer.ts`). Asserting on this rather than
/// on the events is what makes a test say *what is on the screen*.
///
/// `threads` is the sidebar, so an archived conversation is not in it. The
/// client keeps that row cached and renders it under its archive; the sidebar
/// filters it out by `archivedAt` (`SidebarV2.tsx`, `threadSort.ts`). Both
/// halves of that are the client's, and only the first is what these tests are
/// about.
#[derive(Debug, Default, PartialEq, Eq)]
struct Held {
    threads: Vec<String>,
    projects: Vec<String>,
    sequence: i64,
}

impl Held {
    /// What the snapshot a subscription opens with leaves the client holding.
    async fn opening(pool: &PgPool, viewer: &ScopeContext) -> Self {
        let snapshot = shell_snapshot(
            &moatless_backend::t3code::conversations_over(pool),
            &moatless_backend::t3code::conversations_over(pool),
            &defaults(),
            viewer,
        )
        .await
        .unwrap();
        let mut held = Held {
            sequence: snapshot.sequence,
            ..Held::default()
        };
        held.apply(&json!({ "kind": "snapshot", "snapshot": snapshot.value }));
        held
    }

    fn apply(&mut self, item: &Value) {
        assert_decodable_item(item);
        let ids = |rows: &Value| -> Vec<String> {
            rows.as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap().to_string())
                .collect()
        };
        if item["kind"] == "snapshot" {
            // Applied whole and unconditionally, position not compared.
            self.threads = ids(&item["snapshot"]["threads"]);
            self.projects = ids(&item["snapshot"]["projects"]);
            self.sequence = item["snapshot"]["snapshotSequence"].as_i64().unwrap();
            return;
        }
        let sequence = item["sequence"].as_i64().unwrap();
        if sequence <= self.sequence {
            // The client's own rule, and the reason two events may not share a
            // position: the second is discarded without a word.
            return;
        }
        self.sequence = sequence;
        let upsert = |set: &mut Vec<String>, id: &str| {
            if !set.iter().any(|held| held == id) {
                set.push(id.to_string());
            }
        };
        match item["kind"].as_str().unwrap() {
            "thread-upserted" => {
                let id = item["thread"]["id"].as_str().unwrap();
                if item["thread"]["archivedAt"].is_null() {
                    upsert(&mut self.threads, id);
                } else {
                    self.threads.retain(|held| held != id);
                }
            }
            "thread-removed" => self
                .threads
                .retain(|id| id != item["threadId"].as_str().unwrap()),
            "project-upserted" => {
                upsert(&mut self.projects, item["project"]["id"].as_str().unwrap());
            }
            "project-removed" => self
                .projects
                .retain(|id| id != item["projectId"].as_str().unwrap()),
            _ => unreachable!(),
        }
    }

    /// Poll and fold, returning what was delivered.
    async fn poll(
        &mut self,
        pool: &PgPool,
        feed: &mut ShellFeed,
        viewer: &ScopeContext,
    ) -> Vec<Value> {
        let items = delivered(pool, feed, viewer).await;
        for item in &items {
            self.apply(item);
        }
        items
    }
}

/// A watching client, and the listing it is holding, together.
async fn watcher(pool: &PgPool, viewer: &ScopeContext) -> (ShellFeed, Held) {
    (
        watching(pool, viewer).await,
        Held::opening(pool, viewer).await,
    )
}

/// The one change kind that alters a sidebar's *membership* rather than the
/// contents of a row already in it, so a client that handles every other kind
/// can still miss this one entirely.
#[sqlx::test]
async fn following_a_conversation_makes_it_appear_and_unfollowing_removes_it(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    // Readable but not followed, so it is absent from the opening listing —
    // which is what makes the appearance below a change of membership.
    unfollow(&pool, "user-1", "t-live").await;

    let viewer = viewer("user-1");
    let (mut feed, mut held) = watcher(&pool, &viewer).await;
    assert!(held.threads.is_empty(), "nothing is followed yet");

    follow(&pool, "user-1", "t-live").await;
    held.poll(&pool, &mut feed, &viewer).await;
    assert_eq!(
        held.threads,
        ["t-live"],
        "following puts the row on the screen"
    );

    unfollow(&pool, "user-1", "t-live").await;
    let item = one_item(&pool, &mut feed, &viewer).await;
    assert_eq!(item["kind"], "thread-removed");
    assert_eq!(item["threadId"], "t-live");
    held.apply(&item);
    assert!(held.threads.is_empty(), "unfollowing takes it off again");
}

/// The disclosure rule, asserted rather than described.
///
/// A conversation leaves someone's listing, and every other watching client
/// re-reads it and finds nothing — indistinguishable, from the re-read alone,
/// from a conversation they never had. T3's own server emits `thread-removed`
/// on that empty re-read (`apps/server/src/ws.ts`), which on a shared
/// deployment hands every connected client the id of every conversation that
/// was unfollowed, made private or deleted anywhere.
///
/// The watcher here holds a conversation of their own, so an implementation
/// that emitted removals only when it held *nothing* would still fail this.
#[sqlx::test]
async fn a_removal_names_only_a_conversation_this_client_was_sent(pool: PgPool) {
    seed_live_project(&pool).await;
    // `user-2` follows one conversation and can read neither of `user-1`'s.
    seed_task(&pool, "t-theirs", "user-1", Some("ws-live")).await;
    seed_task(&pool, "t-deleted", "user-1", Some("ws-live")).await;
    let mut mine = make_task_row("t-mine", "user-2");
    mine.workspace_id = Some("ws-live".to_string());
    mine.visibility = Visibility::Public;
    moatless_tasks::task::dao::store_over(&pool)
        .insert_task(&mine)
        .await
        .unwrap();

    let watcher_viewer = viewer("user-2");
    let (mut feed, mut held) = watcher(&pool, &watcher_viewer).await;
    assert_eq!(
        held.threads,
        ["t-mine"],
        "they hold their own and nothing else"
    );
    let watched_from = moatless_backend::t3code::conversations_over(&pool)
        .listing_head()
        .await
        .unwrap()
        .position;

    // Two ways a row leaves someone else's listing, neither of them this
    // viewer's business.
    unfollow(&pool, "user-1", "t-theirs").await;
    assert!(
        moatless_tasks::task::dao::store_over(&pool)
            .delete_task("t-deleted", &viewer("user-1"))
            .await
            .unwrap()
    );
    // The premise: the changes really were recorded, so silence below is the
    // feed filtering rather than nothing having happened.
    assert_eq!(recorded_after(&pool, watched_from).await, 2);

    assert_eq!(
        delivered(&pool, &mut feed, &watcher_viewer).await,
        Vec::<Value>::new(),
        "a client was told the id of a conversation it was never sent"
    );
    assert_eq!(held.threads, ["t-mine"]);
    held.poll(&pool, &mut feed, &watcher_viewer).await;
}

#[sqlx::test]
async fn deleting_a_conversation_removes_it_from_a_watching_client(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let (mut feed, mut held) = watcher(&pool, &viewer).await;
    assert_eq!(held.threads, ["t-live"]);

    assert!(
        moatless_tasks::task::dao::store_over(&pool)
            .delete_task("t-live", &viewer)
            .await
            .unwrap()
    );

    let item = one_item(&pool, &mut feed, &viewer).await;
    assert_eq!(item["kind"], "thread-removed");
    assert_eq!(item["threadId"], "t-live");
    held.apply(&item);
    assert!(held.threads.is_empty());
}

/// Closing is the way a row leaves that a person performs on purpose, and the
/// one that comes back: the conversation is not deleted and the follow survives
/// it, so reopening puts the row on the screen again.
///
/// A watching client is told, rather than left holding a conversation the next
/// full listing would not contain — the drift that shows as a sidebar row that
/// only disappears on reload.
///
/// It is told with the row rather than with a removal, because the conversation
/// has not left the viewer's world: they still follow it and may still read it,
/// and `archivedAt` is what puts it under the archive instead of the sidebar. A
/// removal would say something stronger and, on a resumed feed, would not be
/// sent at all.
#[sqlx::test]
async fn closing_a_conversation_removes_it_from_a_watching_client(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let (mut feed, mut held) = watcher(&pool, &viewer).await;
    assert_eq!(held.threads, ["t-live"]);

    assert!(
        moatless_tasks::task::dao::store_over(&pool)
            .update_status_if_not_closed("t-live", TaskStatus::Closed, &viewer)
            .await
            .unwrap()
    );

    let item = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(item["thread"]["id"], "t-live");
    assert!(!item["thread"]["archivedAt"].is_null());
    held.apply(&item);
    assert!(held.threads.is_empty());

    moatless_tasks::task::dao::store_over(&pool)
        .update_task(
            "t-live",
            &TaskPatch {
                status: Some(TaskStatus::Open),
                ..TaskPatch::default()
            },
            &viewer,
        )
        .await
        .unwrap();

    let item = one_row(&pool, &mut feed, &viewer).await;
    assert_eq!(
        item["thread"]["id"], "t-live",
        "reopening puts the row back"
    );
    held.apply(&item);
    assert_eq!(held.threads, ["t-live"]);
}

/// Losing read access is the fourth way a row leaves, and the one no mutation of
/// the conversation's own row expresses: the follow survives, the conversation
/// survives, and it still has to go.
#[sqlx::test]
async fn a_conversation_that_stops_being_readable_is_removed(pool: PgPool) {
    seed_live_project(&pool).await;
    let mut shared = make_task_row("t-shared", "user-1");
    shared.workspace_id = Some("ws-live".to_string());
    shared.visibility = Visibility::Public;
    moatless_tasks::task::dao::store_over(&pool)
        .insert_task(&shared)
        .await
        .unwrap();
    follow(&pool, "user-2", "t-shared").await;

    let watcher_viewer = viewer("user-2");
    let (mut feed, mut held) = watcher(&pool, &watcher_viewer).await;
    assert_eq!(held.threads, ["t-shared"]);

    moatless_tasks::task::dao::store_over(&pool)
        .update_task(
            "t-shared",
            &TaskPatch {
                visibility: Some(Visibility::Private),
                ..TaskPatch::default()
            },
            &viewer("user-1"),
        )
        .await
        .unwrap();

    let item = one_item(&pool, &mut feed, &watcher_viewer).await;
    assert_eq!(item["kind"], "thread-removed");
    assert_eq!(item["threadId"], "t-shared");
    held.apply(&item);
    assert!(held.threads.is_empty());
}

/// Every remaining conversation change kind, each one asserted to move the row
/// a watching client holds — and to move the *value* that kind changes, so a
/// site that recorded a change without writing anything would not pass.
#[sqlx::test]
async fn every_remaining_change_kind_moves_the_row(pool: PgPool) {
    seed_live_project(&pool).await;
    sqlx::query("INSERT INTO tags (tag_id, name) VALUES ('tag-1', 'urgent')")
        .execute(&pool)
        .await
        .unwrap();

    let viewer = viewer("user-1");
    let owner = viewer.clone();

    // Each case: what a person did, and what the delivered row must say
    // afterwards. `None` asserts only that the row was delivered — the tag set
    // and the expiry are not values the shell row carries, and a change to them
    // still has to move the conversation in the sidebar.
    #[allow(clippy::type_complexity)]
    let cases: &[(
        &str,
        fn(PgPool, ScopeContext, String) -> BoxFuture<'static, ()>,
        Option<(&str, Value)>,
    )] = &[
        (
            "renamed",
            |pool, ctx, id| {
                Box::pin(async move {
                    let patch = TaskPatch {
                        name: Some("a new name".into()),
                        ..TaskPatch::default()
                    };
                    moatless_tasks::task::dao::store_over(&pool)
                        .update_task(&id, &patch, &ctx)
                        .await
                        .unwrap();
                })
            },
            Some(("title", json!("a new name"))),
        ),
        (
            "described",
            |pool, ctx, id| {
                Box::pin(async move {
                    let patch = TaskPatch {
                        description: Some("a description".into()),
                        ..TaskPatch::default()
                    };
                    moatless_tasks::task::dao::store_over(&pool)
                        .update_task(&id, &patch, &ctx)
                        .await
                        .unwrap();
                })
            },
            None,
        ),
        // Closing and reopening are deliberately not here: they change the
        // sidebar's *membership* rather than a row in it, and are asserted as
        // such in `closing_a_conversation_removes_it_from_a_watching_client`.
        (
            "given a new expiry",
            |pool, ctx, id| {
                Box::pin(async move {
                    let patch = TaskPatch {
                        ttl_minutes: Some(240),
                        ..TaskPatch::default()
                    };
                    moatless_tasks::task::dao::store_over(&pool)
                        .update_task(&id, &patch, &ctx)
                        .await
                        .unwrap();
                })
            },
            None,
        ),
        (
            "tagged",
            |pool, _ctx, id| {
                Box::pin(async move {
                    PgTagDao(pool)
                        .add_task_tags(&id, &["tag-1".to_string()])
                        .await
                        .unwrap();
                })
            },
            None,
        ),
        (
            "given a new tag set",
            |pool, _ctx, id| {
                Box::pin(async move {
                    let tags = PgTagDao(pool);
                    // Replacing nothing with nothing moves no sidebar and is
                    // deliberately not recorded, so this case has to start from
                    // a tag there is something to replace.
                    tags.add_task_tags(&id, &["tag-1".to_string()])
                        .await
                        .unwrap();
                    tags.set_task_tags(&id, &[]).await.unwrap();
                })
            },
            None,
        ),
        (
            "written to by a person",
            |pool, _ctx, id| {
                Box::pin(async move {
                    let msg = make_message_row(&id, &format!("{id}-said"));
                    PgMessageDao(pool)
                        .append(&msg, &[], &ConversationWrite::default())
                        .await
                        .unwrap();
                })
            },
            // The activity timestamp T3 orders the sidebar by. It was null on
            // a conversation nobody had written to yet.
            Some(("hasPendingUserInput", json!(false))),
        ),
        (
            "asked a question",
            |pool, _ctx, id| {
                Box::pin(async move {
                    let uuid = format!("{id}-asked");
                    let msg = TaskMessage {
                        message_type: MessageType::Assistant,
                        message_text: Some("Which one?".into()),
                        ..make_message_row(&id, &uuid)
                    };
                    let question = map_content(
                        BlockContext {
                            task_id: &id,
                            message_uuid: &uuid,
                            parent_uuid: None,
                            message_type: MessageType::Assistant,
                            block_index: 1,
                        },
                        &json!([{
                            "type": "tool_use",
                            "id": "tu-1",
                            "name": "AskUserQuestion",
                            "input": {}
                        }]),
                    );
                    PgMessageDao(pool)
                        .append(&msg, &[], &question)
                        .await
                        .unwrap();
                })
            },
            Some(("hasPendingUserInput", json!(true))),
        ),
    ];

    for (index, (what, run, expected)) in cases.iter().enumerate() {
        let task_id = format!("t-{index}");
        seed_task(&pool, &task_id, "user-1", Some("ws-live")).await;

        // Watch from after the seeding, so the only change the feed can carry
        // is the one this case makes.
        let mut feed = watching(&pool, &viewer).await;
        let watched_from = moatless_backend::t3code::conversations_over(&pool)
            .listing_head()
            .await
            .unwrap()
            .position;
        run(pool.clone(), owner.clone(), task_id.clone()).await;
        assert!(
            recorded_after(&pool, watched_from).await > 0,
            "a conversation `{what}` recorded no listing change, \
             so nothing about it reaches a sidebar"
        );

        let event = one_row(&pool, &mut feed, &viewer).await;
        assert_eq!(
            event["thread"]["id"], task_id,
            "a conversation `{what}` delivered someone else's row"
        );
        if let Some((key, value)) = expected {
            assert_eq!(
                event["thread"][key], *value,
                "a conversation `{what}` delivered a row whose `{key}` had not moved"
            );
        }
    }
}

/// The acceptance criterion the message-send path exists to satisfy, in the
/// order `message::handlers` performs it: follow first, write second.
///
/// The two changes land between the same pair of polls and coalesce into one
/// re-read, so what the client gets is the row *and* the message, never the
/// message against a row it does not hold.
#[sqlx::test]
async fn writing_to_a_conversation_the_author_did_not_follow_adds_the_row(pool: PgPool) {
    // Deployment-scoped, so `user-2` may read the project as well as the
    // conversation — the case where a delta is enough.
    let mut shared_project = make_workspace_row("ws-shared", "user-1");
    shared_project.scope = Scope::Global;
    shared_project.owner_user_id = None;
    shared_project.created_by = None;
    PgWorkspaceDao(pool.clone())
        .insert_workspace(&shared_project)
        .await
        .unwrap();
    let mut theirs = make_task_row("t-theirs", "user-1");
    theirs.workspace_id = Some("ws-shared".to_string());
    theirs.visibility = Visibility::Public;
    moatless_tasks::task::dao::store_over(&pool)
        .insert_task(&theirs)
        .await
        .unwrap();

    let author = viewer("user-2");
    let (mut feed, mut held) = watcher(&pool, &author).await;
    assert!(held.threads.is_empty(), "they do not follow it yet");
    assert_eq!(held.projects, ["ws-shared"]);

    // The order `send_message` writes in.
    follow(&pool, "user-2", "t-theirs").await;
    seed_user_message(&pool, "t-theirs", "m-1").await;

    let items = held.poll(&pool, &mut feed, &author).await;
    assert_eq!(items.len(), 1, "one row, carrying both changes: {items:#?}");
    assert_eq!(items[0]["kind"], "thread-upserted");
    assert_eq!(held.threads, ["t-theirs"]);
    assert_eq!(
        items[0]["thread"]["projectId"], "ws-shared",
        "the row arrived under a project the client is holding, or it is dropped"
    );
    assert!(
        !items[0]["thread"]["latestUserMessageAt"].is_null(),
        "the message the author just wrote is on the row"
    );
}

/// The same criterion where a delta cannot express it: the conversation is
/// readable and its project is not, so the client has never been sent the
/// project the row needs to hang under.
///
/// Two events at one position deliver one — the client advances its position on
/// the first and discards the second (`shellReducer.ts:15`) — and there is no
/// position below the conversation's to put the project at, because the
/// conversation's change may sit immediately after the last one delivered. So
/// the feed sends the listing whole instead, which the contract declares as a
/// stream item and the client applies unconditionally.
///
/// Against the obvious implementation — emit the `thread-upserted` alone — this
/// test fails on the project assertion, and a real client would drop the row
/// with nothing logged.
#[sqlx::test]
async fn a_conversation_entering_a_project_the_client_lacks_arrives_as_a_listing(pool: PgPool) {
    seed_workspaces(&pool).await;
    let mut theirs = make_task_row("t-theirs", "user-2");
    // `ws-other` belongs to `user-2` and `user-1` cannot read it.
    theirs.workspace_id = Some("ws-other".to_string());
    theirs.visibility = Visibility::Public;
    // `tasks_owner_scope_chk`: a deployment-scoped row has no owner.
    theirs.scope = Scope::Global;
    theirs.owner_user_id = None;
    moatless_tasks::task::dao::store_over(&pool)
        .insert_task(&theirs)
        .await
        .unwrap();

    let watcher_viewer = viewer("user-1");
    let (mut feed, mut held) = watcher(&pool, &watcher_viewer).await;
    assert!(held.threads.is_empty());
    assert!(
        !held.projects.contains(&"ws-other".to_string()),
        "the private project is not in their opening listing"
    );

    follow(&pool, "user-1", "t-theirs").await;

    let item = one_item(&pool, &mut feed, &watcher_viewer).await;
    assert_eq!(
        item["kind"], "snapshot",
        "a row entering an unheld project has to arrive with the project"
    );
    held.apply(&item);
    assert_eq!(held.threads, ["t-theirs"]);
    assert!(
        held.projects.contains(&"ws-other".to_string()),
        "the client holds the project the row hangs under"
    );

    // And the rebuilt listing left the feed caught up rather than repeating
    // itself on the next poll.
    assert!(
        delivered(&pool, &mut feed, &watcher_viewer)
            .await
            .is_empty()
    );
}

/// A long agent turn is the highest-rate path in the system and the one this
/// design is affordable because of. Nothing it produces is a listing change, so
/// the sidebar does not reorder and the deployment's one counter does not move.
#[sqlx::test]
async fn a_long_agent_turn_does_not_reorder_the_sidebar(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    seed_task(&pool, "t-other", "user-1", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let (mut feed, held) = watcher(&pool, &viewer).await;
    let before = moatless_backend::t3code::conversations_over(&pool)
        .listing_head()
        .await
        .unwrap()
        .position;

    // Forty messages of agent output and tool results, the shape of one long
    // turn. Deliberately not the turn's start or end — ticket 04 records those,
    // and they are the whole of what a turn contributes.
    for step in 0..20 {
        let uuid = format!("a-{step}");
        seed_agent_message(&pool, "t-live", &uuid, 1, Some("claude-opus-4-5")).await;
        // The call and the result it comes back with, on the turn that made it.
        let work = map_content(
            BlockContext {
                task_id: "t-live",
                message_uuid: &uuid,
                parent_uuid: None,
                message_type: MessageType::Assistant,
                block_index: 1,
            },
            &json!([
                {"type": "tool_use", "id": format!("tu-{step}"), "name": "Bash", "input": {}},
                {"type": "tool_result", "tool_use_id": format!("tu-{step}"), "content": "ok"}
            ]),
        );
        PgMessageDao(pool.clone())
            .append_activities("t-live", &uuid, &work)
            .await
            .unwrap();
    }

    assert_eq!(
        moatless_backend::t3code::conversations_over(&pool)
            .listing_head()
            .await
            .unwrap()
            .position,
        before,
        "the agent's own output advanced the deployment's position counter"
    );
    assert!(
        delivered(&pool, &mut feed, &viewer).await.is_empty(),
        "the agent's own output reordered a sidebar"
    );
    assert_eq!(held.threads.len(), 2);
}

// -------------------------------------------------------------------------
// Projects
// -------------------------------------------------------------------------

#[sqlx::test]
async fn renaming_a_project_waits_for_the_next_snapshot(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;

    let viewer = viewer("user-1");
    let (mut feed, held) = watcher(&pool, &viewer).await;
    assert_eq!(held.projects, ["ws-live"]);
    let watched_from = head(&pool).await.position;

    assert!(
        PgWorkspaceDao(pool.clone())
            .update(
                "ws-live",
                &WorkspacePatch {
                    name: Some("Renamed project".into()),
                    ..WorkspacePatch::default()
                },
                &viewer,
            )
            .await
            .unwrap()
    );

    assert_eq!(recorded_after(&pool, watched_from).await, 0);
    assert!(delivered(&pool, &mut feed, &viewer).await.is_empty());

    // Durable, not just delivered: a client opening a fresh subscription reads
    // the same title out of the database.
    let reopened = project(&pool, &viewer, "ws-live").await;
    assert_eq!(reopened["title"], "Renamed project");
}

#[sqlx::test]
async fn creating_and_deleting_a_project_waits_for_the_next_snapshot(pool: PgPool) {
    seed_live_project(&pool).await;

    let viewer = viewer("user-1");
    let (mut feed, held) = watcher(&pool, &viewer).await;
    assert_eq!(held.projects, ["ws-live"]);
    let watched_from = head(&pool).await.position;

    PgWorkspaceDao(pool.clone())
        .insert_workspace(&make_workspace_row("ws-new", "user-1"))
        .await
        .unwrap();

    assert!(
        PgWorkspaceDao(pool.clone())
            .soft_delete("ws-new", &viewer)
            .await
            .unwrap()
    );

    assert_eq!(recorded_after(&pool, watched_from).await, 0);
    assert!(delivered(&pool, &mut feed, &viewer).await.is_empty());
    assert_eq!(Held::opening(&pool, &viewer).await.projects, ["ws-live"]);
}

/// The second way a project leaves the listing, which recorded nothing.
///
/// Overriding a git-declared workspace tombstones it: the row survives so a
/// later sync will not recreate it, `source` flips to `manual`, and `deleted`
/// goes true. `WorkspaceDao::list` still returns it — that is what a
/// tombstone is for — but `list_readable_including` does not, and the shell
/// listing is built from the second. So the group comes off the screen exactly
/// as an ordinary delete takes it off, and until this recorded, it came off
/// with nobody told: the client kept rendering the group until something
/// unrelated moved its listing.
#[sqlx::test]
async fn tombstoning_a_git_declared_project_waits_for_the_next_snapshot(pool: PgPool) {
    let repo = PgWorkspaceDao(pool.clone());
    let mut declared = make_workspace_row("ws-git", "user-1");
    declared.source = moatless_types::workspace::WorkspaceSource::Git;
    declared.source_repository_id = Some("repo-1".to_string());
    declared.source_key = Some("api".to_string());
    repo.insert_workspace(&declared).await.unwrap();

    let viewer = viewer("user-1");
    let (mut feed, held) = watcher(&pool, &viewer).await;
    assert_eq!(held.projects, ["ws-git"]);
    let watched_from = head(&pool).await.position;

    assert!(
        repo.soft_delete_git_origin("ws-git", &viewer)
            .await
            .unwrap()
    );
    assert_eq!(
        recorded_after(&pool, watched_from).await,
        0,
        "a Workspace tombstone advanced the Task listing"
    );
    assert!(delivered(&pool, &mut feed, &viewer).await.is_empty());
    assert!(Held::opening(&pool, &viewer).await.projects.is_empty());
}

/// Deleting a repository bulk-deletes every workspace its git tree declared,
/// because once the source repo is gone no future sync will prune them. The
/// mutation is Workspace-only, so live clients retain their current groups
/// until a later snapshot replaces the listing.
///
/// Driven through the mounted HTTP route rather than the query underneath it:
/// the statement lives in a handler, and a test that called the handler's
/// helper directly would pass against a route that never reaches it.
#[sqlx::test]
async fn deleting_a_repository_does_not_advance_the_task_listing(pool: PgPool) {
    seed_team(&pool, &format!("personal-{USER_1}")).await;
    repository_dao::insert(&pool, &make_repository_row("repo-gone", USER_1))
        .await
        .unwrap();

    let workspaces = PgWorkspaceDao(pool.clone());
    for key in ["api", "web"] {
        let mut declared = make_workspace_row(&format!("ws-{key}"), USER_1);
        declared.source = moatless_types::workspace::WorkspaceSource::Git;
        declared.source_repository_id = Some("repo-gone".to_string());
        declared.source_key = Some(key.to_string());
        workspaces.insert_workspace(&declared).await.unwrap();
    }
    // A workspace of the viewer's that the repository did not declare, so the
    // assertion below is "the declared ones went" rather than "everything went".
    workspaces
        .insert_workspace(&make_workspace_row("ws-kept", USER_1))
        .await
        .unwrap();

    let viewer = viewer(USER_1);
    let (mut feed, mut held) = watcher(&pool, &viewer).await;
    held.projects.sort();
    assert_eq!(held.projects, ["ws-api", "ws-kept", "ws-web"]);
    let watched_from = head(&pool).await.position;

    let (router, _api) = repository_router(test_app_state(pool.clone()));
    let response = router
        .oneshot(authed_request(
            "DELETE",
            "/api/v1/repositories/repo-gone",
            USER_1,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        recorded_after(&pool, watched_from).await,
        0,
        "repository-driven Workspace deletion advanced the Task listing"
    );

    let items = held.poll(&pool, &mut feed, &viewer).await;
    assert!(
        items.is_empty(),
        "Workspace deletion emitted shell items: {items:#?}"
    );
    assert_eq!(held.projects, ["ws-api", "ws-kept", "ws-web"]);
    assert_eq!(Held::opening(&pool, &viewer).await.projects, ["ws-kept"]);
}

/// A Workspace someone else owns changes without entering the Task feed.
#[sqlx::test]
async fn a_project_change_reaches_only_the_clients_holding_it(pool: PgPool) {
    seed_workspaces(&pool).await;

    let watcher_viewer = viewer("user-1");
    let (mut feed, held) = watcher(&pool, &watcher_viewer).await;
    assert!(
        !held.projects.contains(&"ws-other".to_string()),
        "`ws-other` belongs to `user-2`"
    );
    let watched_from = moatless_backend::t3code::conversations_over(&pool)
        .listing_head()
        .await
        .unwrap()
        .position;

    assert!(
        PgWorkspaceDao(pool.clone())
            .update(
                "ws-other",
                &WorkspacePatch {
                    name: Some("Someone else's".into()),
                    ..WorkspacePatch::default()
                },
                &viewer("user-2"),
            )
            .await
            .unwrap()
    );
    assert!(
        PgWorkspaceDao(pool.clone())
            .soft_delete("ws-other", &viewer("user-2"))
            .await
            .unwrap()
    );

    assert_eq!(recorded_after(&pool, watched_from).await, 0);
    assert_eq!(
        delivered(&pool, &mut feed, &watcher_viewer).await,
        Vec::<Value>::new(),
        "a client was told about a project it cannot see"
    );
}

/// A project a viewer cannot read but holds because they follow work inside it
/// is still snapshot-owned. Its mutations emit no live project events.
#[sqlx::test]
async fn a_referenced_project_change_waits_for_the_next_snapshot(pool: PgPool) {
    seed_workspaces(&pool).await;
    let mut theirs = make_task_row("t-theirs", "user-2");
    theirs.workspace_id = Some("ws-other".to_string());
    theirs.visibility = Visibility::Public;
    // `tasks_owner_scope_chk`: a deployment-scoped row has no owner.
    theirs.scope = Scope::Global;
    theirs.owner_user_id = None;
    moatless_tasks::task::dao::store_over(&pool)
        .insert_task(&theirs)
        .await
        .unwrap();
    follow(&pool, "user-1", "t-theirs").await;
    // Something for the narrowing to withhold: a `scripts` assertion against a
    // workspace with no scripts passes whatever the projection does.
    assert!(
        PgWorkspaceDao(pool.clone())
            .update(
                "ws-other",
                &WorkspacePatch {
                    scripts: Some(Some(run_config(json!([{
                        "name": "dev",
                        "command": "bun run secret-thing"
                    }])))),
                    ..WorkspacePatch::default()
                },
                &viewer("user-2"),
            )
            .await
            .unwrap()
    );

    let watcher_viewer = viewer("user-1");
    let (mut feed, mut held) = watcher(&pool, &watcher_viewer).await;
    assert!(
        held.projects.contains(&"ws-other".to_string()),
        "the widening half put an unreadable project in their listing"
    );
    let watched_from = head(&pool).await.position;

    assert!(
        PgWorkspaceDao(pool.clone())
            .update(
                "ws-other",
                &WorkspacePatch {
                    name: Some("Renamed elsewhere".into()),
                    ..WorkspacePatch::default()
                },
                &viewer("user-2"),
            )
            .await
            .unwrap()
    );

    assert!(
        PgWorkspaceDao(pool.clone())
            .soft_delete("ws-other", &viewer("user-2"))
            .await
            .unwrap()
    );
    let items = held.poll(&pool, &mut feed, &watcher_viewer).await;
    assert!(
        items.is_empty(),
        "Workspace changes emitted shell items: {items:#?}"
    );
    assert_eq!(recorded_after(&pool, watched_from).await, 0);
    assert!(held.projects.contains(&"ws-other".to_string()));
    assert!(
        !Held::opening(&pool, &watcher_viewer)
            .await
            .projects
            .contains(&"ws-other".to_string())
    );
}

// =========================================================================
// What survives the merge of 05 and 06
// =========================================================================
//
// The opening snapshot answers both which Tasks are held and which projects can
// group them. A resumed feed reconstructs only the grouping half.

/// A client that reconnects, catches up on nothing, and then watches one
/// conversation change.
///
/// The merge's headline defect. `ShellFeed::resuming_from` leaves the id sets
/// empty, and `poll` treats a conversation whose project the client is not known
/// to hold as grounds to send the listing whole — so on the merged branch, and
/// before this ticket, the *first* live change after any reconnection arrived as
/// a full snapshot. That is the cost ticket 06 exists to remove, reintroduced
/// behind it.
///
/// Deliberately separate from the catch-up: this client had nothing to replay,
/// so the empty first chunk is not what is being tested. The break it catches is
/// a `grouped` set that a resume leaves empty.
#[sqlx::test]
async fn a_resumed_client_watches_the_next_change_as_a_row_not_a_listing(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");

    // It read the listing, the socket dropped, and nothing happened meanwhile.
    let held = head(&pool).await.position;
    let (values, mut feed) = reconnect(&pool, &viewer, Some(held)).await;
    assert!(
        values.is_empty(),
        "the premise failed: this client was not caught up, so the first live \
         poll is not what is being measured: {values:#?}"
    );

    // One turn, on a conversation in a project it already had.
    let turn = start_turn(&pool, "t-live", "run-1").await;
    ingress(&pool, "t-live", turn, "agent", "completed").await;

    let items = delivered(&pool, &mut feed, &viewer).await;
    assert!(
        !items.iter().any(|item| item["kind"] == "snapshot"),
        "a resumed client was sent its whole listing for one conversation \
         changing: {items:#?}"
    );
    assert_eq!(
        items.len(),
        1,
        "the burst on one conversation coalesces into one row, got {items:#?}"
    );
    assert_decodable_upsert(&items[0]);
    assert_eq!(items[0]["thread"]["id"], "t-live");
    assert_eq!(items[0]["thread"]["latestTurn"]["state"], "completed");
}

/// Archiving reaches a resumed client, which is every client: the T3 web app
/// reads the snapshot over HTTP and then subscribes from its sequence, so the
/// feed behind a live sidebar is always a resumed one.
///
/// A resumed feed holds no ids it can prove it sent, so it may not name one in a
/// removal. Archiving therefore arrives as the row itself, carrying
/// `archivedAt` — which the client filters out of the sidebar and files under
/// the archive. A conversation that left the viewer's world entirely still has
/// nothing to arrive as, and that is the case removals are kept for.
#[sqlx::test]
async fn archiving_reaches_a_resumed_client_as_the_row_it_archived(pool: PgPool) {
    seed_live_project(&pool).await;
    seed_task(&pool, "t-live", "user-1", Some("ws-live")).await;
    let viewer = viewer("user-1");

    let held = head(&pool).await.position;
    let (_, mut feed) = reconnect(&pool, &viewer, Some(held)).await;

    moatless_tasks::task::dao::store_over(&pool)
        .update_task(
            "t-live",
            &TaskPatch {
                status: Some(TaskStatus::Closed),
                ..TaskPatch::default()
            },
            &viewer,
        )
        .await
        .unwrap();

    let items = delivered(&pool, &mut feed, &viewer).await;
    assert_eq!(
        items.len(),
        1,
        "archiving is one event on one conversation, got {items:#?}"
    );
    assert_decodable_upsert(&items[0]);
    assert_eq!(items[0]["thread"]["id"], "t-live");
    assert!(
        !items[0]["thread"]["archivedAt"].is_null(),
        "the archived row must carry the time it was archived, or the client \
         has no reason to take it off the sidebar: {items:#?}"
    );

    // Reopening is the same shape at the other status, so the row comes back
    // with nothing to filter it out by.
    moatless_tasks::task::dao::store_over(&pool)
        .update_task(
            "t-live",
            &TaskPatch {
                status: Some(TaskStatus::Open),
                ..TaskPatch::default()
            },
            &viewer,
        )
        .await
        .unwrap();

    let items = delivered(&pool, &mut feed, &viewer).await;
    assert_eq!(items.len(), 1, "got {items:#?}");
    assert_decodable_upsert(&items[0]);
    assert_eq!(items[0]["thread"]["archivedAt"], Value::Null);
}

/// The delta must not be bought by weakening the case it was carved out of.
///
/// A conversation entering a project the client has no group for still cannot be
/// delivered as two events at one position, so it still arrives as a listing —
/// on a resumed subscription exactly as on a fresh one. This is the test that
/// fails if `grouped` is seeded with something that admits every project rather
/// than the ones this viewer can read in their own right.
#[sqlx::test]
async fn a_resumed_client_gaining_an_unreadable_project_still_gets_the_listing_whole(pool: PgPool) {
    seed_workspaces(&pool).await;
    // `ws-other` belongs to `user-2`, and `user-1` cannot read it.
    let mut theirs = make_task_row("t-theirs", "user-2");
    theirs.workspace_id = Some("ws-other".to_string());
    theirs.visibility = Visibility::Public;
    theirs.scope = Scope::Global;
    theirs.owner_user_id = None;
    moatless_tasks::task::dao::store_over(&pool)
        .insert_task(&theirs)
        .await
        .unwrap();

    let watcher_viewer = viewer("user-1");
    let mut held = Held::opening(&pool, &watcher_viewer).await;
    assert!(
        !held.projects.contains(&"ws-other".to_string()),
        "the premise failed: the private project is in their listing already"
    );

    let (values, mut feed) = reconnect(&pool, &watcher_viewer, Some(held.sequence)).await;
    assert!(values.is_empty());

    follow(&pool, "user-1", "t-theirs").await;

    let item = one_item(&pool, &mut feed, &watcher_viewer).await;
    assert_eq!(
        item["kind"], "snapshot",
        "a resumed client was sent a row under a project it has no group for, \
         and drops it in silence: {item:#?}"
    );
    held.apply(&item);
    assert_eq!(held.threads, ["t-theirs"]);
    assert!(held.projects.contains(&"ws-other".to_string()));
}

/// Acceptance criterion 4: a removal after a resume names only ids that client
/// was sent, exactly as on a fresh subscription.
///
/// A resumed feed starts with an empty removable set — nothing here tells this
/// server which rows it sent before the socket dropped — and grows it by
/// delivery alone. So the property is asserted in both directions on one feed:
/// the ids it has *not* sent stay unmentioned however thoroughly they leave, and
/// the one it *has* sent comes back as a removal naming it and nothing else.
///
/// Verified by seeding the resumed feed's `threads` from the viewer's followed
/// conversations and watching the first assertion go red.
#[sqlx::test]
async fn a_removal_after_a_resume_names_only_ids_that_client_was_sent(pool: PgPool) {
    // A deployment-wide project, so this viewer can read it in their own right
    // and their resumed feed can deliver into it as a row. A project they held
    // only because a conversation referenced it would cost a listing here, and
    // that is a different test.
    seed_workspaces(&pool).await;
    // `user-2` may read neither of `user-1`'s, and follows one of its own.
    seed_task(&pool, "t-theirs", "user-1", Some("ws-global")).await;
    seed_task(&pool, "t-deleted", "user-1", Some("ws-global")).await;
    let mut mine = make_task_row("t-mine", "user-2");
    mine.workspace_id = Some("ws-global".to_string());
    mine.visibility = Visibility::Public;
    moatless_tasks::task::dao::store_over(&pool)
        .insert_task(&mine)
        .await
        .unwrap();

    let watcher_viewer = viewer("user-2");
    let held = head(&pool).await.position;
    let (values, mut feed) = reconnect(&pool, &watcher_viewer, Some(held)).await;
    assert!(values.is_empty());

    // Two ways a row leaves someone else's listing. Neither is this viewer's
    // business, and a resumed feed that had reconstructed a held set from what
    // this viewer may read would still have to stay silent here.
    unfollow(&pool, "user-1", "t-theirs").await;
    assert!(
        moatless_tasks::task::dao::store_over(&pool)
            .delete_task("t-deleted", &viewer("user-1"))
            .await
            .unwrap()
    );
    assert_eq!(recorded_after(&pool, held).await, 2, "the premise failed");
    assert_eq!(
        delivered(&pool, &mut feed, &watcher_viewer).await,
        Vec::<Value>::new(),
        "a resumed client was told the id of a conversation it was never sent"
    );

    // Its own conversation moves, so now the feed has sent it — and only now
    // may it be taken back.
    let turn = start_turn(&pool, "t-mine", "run-1").await;
    let upsert = one_row(&pool, &mut feed, &watcher_viewer).await;
    assert_eq!(upsert["thread"]["id"], "t-mine");

    ingress(&pool, "t-mine", turn, "agent", "completed").await;
    assert!(
        moatless_tasks::task::dao::store_over(&pool)
            .delete_task("t-mine", &watcher_viewer)
            .await
            .unwrap()
    );
    let items = delivered(&pool, &mut feed, &watcher_viewer).await;
    assert_eq!(items.len(), 1, "expected one removal, got {items:#?}");
    assert_keys(&items[0], "thread-removed", THREAD_REMOVED_REQUIRED, &[]);
    assert_eq!(items[0]["kind"], "thread-removed");
    assert_eq!(items[0]["threadId"], "t-mine");
}

/// Acceptance criterion 6: the held set is what the client was *shown*, not what
/// the viewer may *read*.
///
/// The two are different sets and the difference is constructible: a
/// conversation the viewer follows and may read, whose workspace is gone, comes
/// back from `list_followed` and is dropped during projection — so it is in
/// everything the viewer may read and in nothing they were ever shown. A held
/// set taken from the first would later "remove" a row that was never on the
/// screen, which is a `thread-removed` carrying an id and nothing else to a
/// client that has no idea what it refers to.
///
/// Asserted on both subscription shapes, because they seed differently and only
/// one of them was ever in doubt: the opening snapshot reads its ids back out of
/// the projected listing, and the resume seeds nothing at all.
#[sqlx::test]
async fn a_removal_never_names_a_row_the_listing_did_not_show(pool: PgPool) {
    seed_workspaces(&pool).await;
    seed_task(&pool, "t-shown", "user-1", Some("ws-own")).await;
    // `ws-deleted` is soft-deleted, so this conversation is followed, readable,
    // and not listable.
    seed_task(&pool, "t-orphan", "user-1", Some("ws-deleted")).await;
    let watcher_viewer = viewer("user-1");

    // The premise, stated against the query a reconstruction would use.
    let followed = moatless_backend::t3code::conversations_over(&pool)
        .list_followed(&watcher_viewer)
        .await
        .unwrap();
    assert!(
        followed.iter().any(|row| row.task_id == "t-orphan"),
        "the premise failed: the orphan is not among what this viewer may read"
    );
    // Opened through `shell_subscription`, not through this file's helpers, so
    // the seeding under test is the one the server actually does.
    let (opening, mut fresh) = reconnect(&pool, &watcher_viewer, None).await;
    let mut held = Held::default();
    assert_eq!(opening.len(), 1);
    held.apply(&opening[0]);
    assert_eq!(
        held.threads,
        ["t-shown"],
        "the premise failed: the orphan was shown after all"
    );

    let resumed_from = head(&pool).await.position;
    let (values, mut resumed) = reconnect(&pool, &watcher_viewer, Some(resumed_from)).await;
    assert!(values.is_empty());

    // It leaves. Neither feed may name it, because neither client ever saw it.
    assert!(
        moatless_tasks::task::dao::store_over(&pool)
            .delete_task("t-orphan", &watcher_viewer)
            .await
            .unwrap()
    );
    assert_eq!(
        recorded_after(&pool, resumed_from).await,
        1,
        "the premise failed: the deletion recorded nothing to deliver"
    );
    for (what, feed) in [("opening", &mut fresh), ("resumed", &mut resumed)] {
        assert_eq!(
            delivered(&pool, feed, &watcher_viewer).await,
            Vec::<Value>::new(),
            "the {what} subscription took its held set from what this viewer \
             may read rather than from what its client was shown"
        );
    }
}

/// Acceptance criterion 5, and the hazard that goes wrong quietly.
///
/// `rebuild` replaces the client's listing wholesale, so it has to replace what
/// the feed believes about that listing too. Ticket 07 wrote it before ticket 05
/// gave a feed any id sets to replace, and neither branch could see the other:
/// left as it merges, `rebuild` moves the identity and the cursor into the new
/// database and leaves `threads` and project grouping describing a listing that
/// no longer exists anywhere.
///
/// Both directions of that are silent, and both are asserted here against a real
/// second database:
///
/// - A row from the *replaced* listing must not come back as a removal. The
///   client does not have it; the id would refer to nothing.
/// - A row the rebuild snapshot *introduced* must come back as a removal when it
///   goes. Stale sets never contained it, so the client would keep it forever.
///
/// Verified by deleting the two reseeding lines from `ShellFeed::adopt` and
/// watching each half fail on its own.
#[sqlx::test]
async fn a_rebuilt_feed_describes_the_listing_it_sent_not_the_one_it_replaced(pool: PgPool) {
    let watcher_viewer = viewer("user-1");
    seed_live_project(&pool).await;
    seed_task(&pool, "t-in-both", "user-1", Some("ws-live")).await;

    let restored = back_up_and_restore(pool).await;

    // The deployment carries on, and its clients hold rows the copy never got.
    seed_task(&restored.origin, "t-origin-only", "user-1", Some("ws-live")).await;
    other_peoples_work(&restored.origin, 100).await;
    let mut feed = watching(&restored.origin, &watcher_viewer).await;
    assert!(
        delivered(&restored.origin, &mut feed, &watcher_viewer)
            .await
            .is_empty()
    );

    // The copy grows a project and a conversation of its own, which this client
    // has never been sent anything about.
    PgWorkspaceDao(restored.pool.clone())
        .insert_workspace(&make_workspace_row("ws-restore-only", "user-1"))
        .await
        .unwrap();
    seed_task(
        &restored.pool,
        "t-restore-only",
        "user-1",
        Some("ws-restore-only"),
    )
    .await;

    // Repointed. One poll, one rebuilt listing.
    let items = delivered(&restored.pool, &mut feed, &watcher_viewer).await;
    assert_eq!(items.len(), 1, "expected one rebuild, got {items:#?}");
    assert_decodable_snapshot_item(&items[0]);
    let mut held = Held::default();
    held.apply(&items[0]);
    held.threads.sort();
    assert_eq!(held.threads, ["t-in-both", "t-restore-only"]);
    assert!(held.projects.contains(&"ws-restore-only".to_string()));
    assert!(
        !held.threads.contains(&"t-origin-only".to_string()),
        "the premise failed: the copy has the row the client is holding, so a \
         stale set would not be observable"
    );

    // A row from the listing the rebuild replaced. It changes in the database
    // that is actually there now, where it does not exist.
    let mut tx = restored.pool.begin().await.unwrap();
    record_change(&mut tx, "t-origin-only").await.unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        delivered(&restored.pool, &mut feed, &watcher_viewer).await,
        Vec::<Value>::new(),
        "the rebuilt feed took a row back from the listing it had already \
         replaced — the client has no such row, and the id names nothing"
    );

    // A row only the rebuild snapshot introduced, leaving.
    assert!(
        moatless_tasks::task::dao::store_over(&restored.pool)
            .delete_task("t-restore-only", &watcher_viewer)
            .await
            .unwrap()
    );
    let items = delivered(&restored.pool, &mut feed, &watcher_viewer).await;
    assert_eq!(items.len(), 1, "expected one removal, got {items:#?}");
    assert_eq!(items[0]["kind"], "thread-removed");
    assert_eq!(items[0]["threadId"], "t-restore-only");

    // Workspace removal is snapshot-owned and emits no live item.
    assert!(
        PgWorkspaceDao(restored.pool.clone())
            .soft_delete("ws-restore-only", &watcher_viewer)
            .await
            .unwrap()
    );
    let items = delivered(&restored.pool, &mut feed, &watcher_viewer).await;
    assert!(
        items.is_empty(),
        "Workspace removal emitted shell items: {items:#?}"
    );

    restored.discard().await;
}
