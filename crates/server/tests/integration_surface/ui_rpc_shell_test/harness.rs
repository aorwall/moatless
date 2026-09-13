//! Reaching the listing: the compatibility router's dependencies as `main`
//! assembles them, a request that carries both auth extensions, and the
//! accessors every assertion reads a snapshot through.

use crate::common::authed_request;
use crate::common::test_app_state;
use chrono::DateTime;
use chrono::Utc;
use moatless_backend::t3code::backend as t3code_backend;
use moatless_ports::sandbox::ClusterDiagnostics;
use moatless_ports::sandbox::ContractSandboxStatus;
use moatless_ports::sandbox::LiveStatus;
use moatless_ports::sandbox::LogOptions;
use moatless_ports::sandbox::SandboxDiagnosticsEntry;
use moatless_ports::sandbox::SandboxError;
use moatless_ports::sandbox::SandboxPortPublisher;
use moatless_ports::sandbox::ServerLogStream;
use moatless_t3code::AgentModelDefaults;
use moatless_t3code::SandboxConnector;
use moatless_t3code::SandboxIntentReader;
use moatless_t3code::SandboxObserver;
use moatless_t3code::SandboxStarter;
use moatless_t3code::SandboxStopper;
use moatless_t3code::T3Reads;
use moatless_t3code::UiRpcAuthIdentity;
use moatless_t3code::UiRpcDependencies;
use moatless_t3code::UserRole;
use moatless_t3code::archived_shell_snapshot;
use moatless_t3code::listing::UiRpcListingHead;
use moatless_t3code::shell_snapshot;
use moatless_t3code::thread_shell;
use moatless_tasks::message::models::MessageType;
use moatless_tasks::task::models::SandboxDesiredState;
use moatless_tasks::task::models::SandboxStopReason;
use moatless_tasks::task::models::TaskStatus;
use moatless_tasks::task_activities::mapping::BlockContext;
use moatless_tasks::task_activities::mapping::ConversationWrite;
use moatless_tasks::task_activities::mapping::map_content;
use moatless_types::access_control::ScopeContext;
use moatless_workspaces::compose;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;

/// Map one Message's content the way the write path does: text onto the row,
/// each call and each outcome onto the conversation. Tests go through this same
/// seam, rather than handing `append` a block list directly.
pub(super) fn conversation_write(
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
/// The auth middleware inserts `AuthIdentity` and `UiRpcAuthIdentity`
/// separately; bypassing it here means a T3 route given only the first 500s on
/// a missing extension instead of returning the response under test.
pub(super) fn ui_rpc_authed_request(
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

pub(super) fn viewer(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.into(),
        is_admin: false,
    }
}

pub(super) fn admin(user_id: &str) -> ScopeContext {
    ScopeContext {
        user_id: user_id.into(),
        is_admin: true,
    }
}

pub(super) fn defaults() -> AgentModelDefaults {
    AgentModelDefaults {
        claude_code: Some("claude-opus-4-5".into()),
        codex: Some("gpt-5.6-sol".into()),
    }
}

/// A deployment with no Sandboxes running.
///
/// The routes this suite drives never reach one: what a Sandbox answers is a
/// gRPC round trip against a pod or the K8s API, and no test here has either.
pub(super) struct NoSandboxes;

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

    async fn detail(
        &self,
        task_id: &str,
        _task_status: TaskStatus,
        _desired_state: SandboxDesiredState,
    ) -> Result<moatless_ports::sandbox::SandboxDetail, SandboxError> {
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
impl moatless_ports::sandbox::SandboxOperator for NoSandboxes {
    async fn restart(&self, task_id: &str) -> Result<(), SandboxError> {
        Err(SandboxError::NotReady(task_id.to_string()))
    }

    async fn redeploy(&self, task_id: &str) -> Result<(), SandboxError> {
        Err(SandboxError::NotReady(task_id.to_string()))
    }

    async fn cleanup(&self, task_id: &str) -> Result<(), SandboxError> {
        Err(SandboxError::NotReady(task_id.to_string()))
    }

    async fn expected_image(&self, task_id: &str) -> Result<String, SandboxError> {
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

/// The compatibility router as `main` mounts it, past the environment gate.
pub(super) async fn mounted_router(pool: &PgPool) -> axum::Router {
    moatless_t3code::mount(
        &moatless_backend::t3code::t3_graph_over(pool),
        mounted_dependencies(pool),
        moatless_types::auth::parse_allowed_origins("http://localhost:5733"),
    )
    .await
    .unwrap()
    .router
}

/// The compatibility router's dependencies, as `main` assembles them. Pairing
/// itself is asserted in `crates/identity/src/auth/pairing_dao_tests.rs`; here it
/// is wired only because the router carries it.
fn mounted_dependencies(pool: &PgPool) -> UiRpcDependencies {
    UiRpcDependencies {
        session_cookie_name: "moatless_session".into(),
        session_jwt_secret: Some("test-secret".into()),
        pairing: moatless_identity::compose::device_pairing(pool.clone()),
        workspace_script_commands: Arc::new(t3code_backend::WorkspaceScripts(compose::workspaces(
            pool.clone(),
        ))),
        sandboxes: Arc::new(NoSandboxes),
        // This suite drives no snapshot reads; a stopped Task answers as before.
        snapshots: None,
        observer: Arc::new(NoSandboxes),
        diagnostics: Arc::new(NoSandboxes),
        // Nothing this suite drives publishes a Script's port.
        ports: Arc::new(NoSandboxes),
        intent: Arc::new(NoSandboxes),
        // Nothing this suite drives starts or stops a Sandbox.
        starter: Arc::new(NoSandboxes),
        stopper: Arc::new(NoSandboxes),
        operator: Arc::new(NoSandboxes),
        // Nothing this suite drives opens a preview tab. With no Redis URL the
        // builder holds the tabs in this process, which is a production path
        // rather than a double — see the epoch note on `PreviewTabDao`.
        redis_url: None,
        preview_tab_ttl: moatless_t3code::preview_tabs::PreviewTabTtl(Duration::from_hours(1)),
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
        pull_requests: Arc::new(t3code_backend::AppStateBoundPullRequests(
            test_app_state(pool.clone()),
            reads(pool),
        )),
        source_control_auth: Arc::new(t3code_backend::AppStateSourceControlAuth(test_app_state(
            pool.clone(),
        ))),
        commands: Arc::new(t3code_backend::AppStateCommands(test_app_state(
            pool.clone(),
        ))),
        attachments: Arc::new(t3code_backend::AppStateAttachments(
            test_app_state(pool.clone()),
            reads(pool),
        )),
        model_defaults: defaults(),
        frontend_url: "http://localhost:3000".into(),
        otlp_traces_enabled: false,
    }
}

/// The T3 read model over `pool`, as the composition root assembles it.
pub(super) fn reads(pool: &PgPool) -> T3Reads {
    moatless_backend::t3code::t3_graph_over(pool)
        .reads()
        .clone()
}

/// Every assertion below goes through here, so a query Postgres refuses fails
/// with the database's own error text, not as an empty listing.
///
/// The SQL in `dao.rs` is unchecked until a connection sees it, so this is the
/// only place a broken join, decode, or parameter can announce itself.
pub(super) async fn snapshot(pool: &PgPool, viewer: &ScopeContext) -> Value {
    match shell_snapshot(&reads(pool), &defaults(), viewer).await {
        Ok(snapshot) => snapshot.value,
        Err(error) => panic!(
            "the shell listing query failed for viewer {} (is_admin={}): {error}\n\n{error:?}",
            viewer.user_id, viewer.is_admin
        ),
    }
}

/// The project ids in a viewer's listing, sorted for comparison.
pub(super) async fn project_ids(pool: &PgPool, viewer: &ScopeContext) -> Vec<String> {
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
pub(super) async fn archived_snapshot(pool: &PgPool, viewer: &ScopeContext) -> Value {
    archived_shell_snapshot(&reads(pool), &defaults(), viewer)
        .await
        .expect("the archived listing query failed")
}

/// The conversation ids in a viewer's archive, sorted for comparison.
pub(super) async fn archived_thread_ids(pool: &PgPool, viewer: &ScopeContext) -> Vec<String> {
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
pub(super) async fn thread_ids(pool: &PgPool, viewer: &ScopeContext) -> Vec<String> {
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
pub(super) async fn thread_shell_row(
    pool: &PgPool,
    viewer: &ScopeContext,
    id: &str,
) -> Option<Value> {
    thread_shell(&reads(pool), &defaults(), viewer, id)
        .await
        .expect("the listing row query failed")
}

pub(super) async fn thread(pool: &PgPool, viewer: &ScopeContext, id: &str) -> Value {
    snapshot(pool, viewer).await["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["id"] == id)
        .unwrap_or_else(|| panic!("{id} is absent from the listing"))
        .clone()
}

pub(super) async fn project(pool: &PgPool, viewer: &ScopeContext, id: &str) -> Value {
    snapshot(pool, viewer).await["projects"]
        .as_array()
        .unwrap()
        .iter()
        .find(|project| project["id"] == id)
        .unwrap_or_else(|| panic!("{id} is absent from the listing"))
        .clone()
}

pub(super) fn parse(value: &Value) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(value.as_str().unwrap())
        .unwrap()
        .with_timezone(&Utc)
}

pub(super) async fn head(pool: &PgPool) -> UiRpcListingHead {
    reads(pool).listing_head().await.unwrap()
}
