use crate::config::AgentModelDefaults;
use crate::config::T3_CLIENT_CONTRACT_VERSION;
use crate::config::UiRpcConfig;
use crate::config::UiRpcConfigError;
use crate::config::environment_id;
use crate::config::parse_enabled;
use crate::dao::ConversationDao;
use crate::handlers::origin_allowed;
use crate::handlers::origin_checked;
use crate::listing::UiRpcEnvironmentIdentity;
use crate::observability;
use crate::ports;
use crate::preview::PreviewState;
use crate::preview_tabs;
use crate::routes::router;
use crate::rpc::config::descriptor;
use crate::rpc::config::server_config;
use crate::rpc::frames::RequestId;
use crate::rpc::frames::THREAD_UNAVAILABLE;
use crate::rpc::frames::sandbox_failure_exit;
use crate::rpc::frames::sandbox_stream_refusal;
use crate::rpc::frames::scripts_failure_exit;
use crate::rpc::metrics as rpc_metrics;
use crate::rpc::metrics::record_request;
use crate::rpc::metrics::remote_span_context;
use crate::rpc::metrics::request_root_span;
use crate::rpc::session::MAX_ACTIVE_STREAMS;
use crate::rpc::session::ProtocolOutput;
use crate::rpc::session::ProtocolSession;
use crate::rpc::socket::MAX_FRAME_BYTES;
use crate::rpc::socket::OUTBOUND_FRAMES;
use crate::sandbox::SandboxRpcError;
use crate::sandbox::SandboxState;
use crate::scripts::ScriptsError;
use crate::scripts::ScriptsState;
use crate::servers::ServersState;
use crate::snapshots::after_sequence;
use crate::state::UiRpcState;
use crate::subscriptions::thread_initial_values;
use crate::subtasks::SubtasksState;
use crate::terminal;
use crate::workspace::WorkspaceAccessError;
use axum::extract::Extension;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::http::header;
use axum::response::Response;
use moatless_core::change_feed::conversation::ConversationHead;
use moatless_identity::auth::pairing::DevicePairing;
use moatless_ports::sandbox::ClusterDiagnostics;
use moatless_ports::sandbox::SandboxIntentReader;
use moatless_ports::sandbox::SandboxObserver;
use moatless_ports::sandbox::SandboxStarter;
use moatless_ports::sandbox::SandboxStopper;
use moatless_types::access_control::ScopeContext;
use moatless_types::auth::AllowedOrigin;
use moatless_types::ui_rpc::UiRpcAuthIdentity;
use moatless_types::ui_rpc::UiRpcAuthMethod;
use moatless_types::ui_rpc::UserRole;
use sandbox_proto::client::SandboxConnector;
use serde_json::Value;
use serde_json::json;
use std::env;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;
use tokio::sync::mpsc;

use crate::dao::MockConversationDao;
use crate::listing::UiRpcListingHead;
use crate::task_dao::{MockT3TaskDao, TaskView};
use crate::workspace_dao::MockT3WorkspaceDao;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use moatless_identity::auth::pairing::{
    PairedDevice, PairingApiKeyIssuer, PairingCode, PairingCodeDao, PairingError, hash_key,
};
use moatless_ports::sandbox::{
    ContractSandboxStatus, LiveStatus, LogOptions, SandboxDesiredState,
    SandboxDiagnosticsContainer, SandboxDiagnosticsEntry, SandboxError, SandboxPortPublisher,
    SandboxStatus, ServerLogStream,
};
use moatless_types::task::TaskStatus;
use uuid::Uuid;

/// The stub's database. Every read in these tests reports it, so nothing
/// here exercises the identity guard — that needs two real databases, and
/// is in `backend/tests/integration_surface/ui_rpc_shell_test.rs`.
fn stub_identity() -> UiRpcEnvironmentIdentity {
    UiRpcEnvironmentIdentity::new("7667539899605483559:16384")
}

/// A stopped Sandbox is the one `scripts.run` refusal the client answers by
/// itself — it starts the Sandbox and runs the Script again — so it goes out
/// as a `Fail` carrying `SandboxNotRunningError`, not a `Die` carrying a
/// sentence. The tag and the `threadId` are the contract
/// (`packages/contracts/src/sandbox.ts`); a `Die` here would put auto-start
/// behind string matching.
#[test]
fn a_stopped_sandbox_refuses_a_script_run_in_terms_the_client_can_act_on() {
    let identity = viewer();
    let payload = json!({ "threadId": "task-1", "scriptId": "dev" });
    let error = ScriptsError::Port(SandboxError::NotReady("stopped".to_string()));

    let request_id: RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
    let exit = scripts_failure_exit(&request_id, &error, &payload, &identity);

    let cause = &exit["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail", "a Die would not decode client-side");
    assert_eq!(cause["error"]["_tag"], "SandboxNotRunningError");
    assert_eq!(cause["error"]["threadId"], "task-1");
}

/// A reserved port is the person's configuration to fix, not the client's —
/// it stays a plain refusal, so the typed arm above must not swallow every
/// port failure.
#[test]
fn a_reserved_port_is_still_reported_rather_than_retried() {
    let identity = viewer();
    let payload = json!({ "threadId": "task-1", "scriptId": "dev" });
    let error = ScriptsError::Port(SandboxError::BadRequest("Port 8001 is reserved".into()));

    let request_id: RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
    let exit = scripts_failure_exit(&request_id, &error, &payload, &identity);

    assert_eq!(exit["exit"]["cause"][0]["_tag"], "Die");
}

/// A Script that publishes no port reaches a stopped Sandbox through the
/// terminal it would have run in, and that path has to answer with the same
/// typed refusal the port path does. `scripts.run` declares
/// `SandboxNotRunningError` and not the `Terminal*` tags, so a
/// `TerminalNotRunningError` here fails the client's own decode and the run ends
/// in a schema error instead of an auto-start.
#[test]
fn a_stopped_sandbox_refuses_a_portless_script_run_in_the_same_terms() {
    let identity = viewer();
    let payload = json!({ "threadId": "task-1", "scriptId": "dev" });
    let error = ScriptsError::Terminal(terminal::TerminalError::Workspace(
        WorkspaceAccessError::NotRunning(SandboxError::NotReady("stopped".to_string())),
    ));

    let request_id: RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
    let exit = scripts_failure_exit(&request_id, &error, &payload, &identity);

    let cause = &exit["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail", "a Die would not auto-start");
    assert_eq!(cause["error"]["_tag"], "SandboxNotRunningError");
    assert_eq!(cause["error"]["threadId"], "task-1");
}

/// Every other terminal failure is outside this method's declared union, so it
/// goes out as a `Die` the client reports rather than a `Fail` it cannot decode.
#[test]
fn a_terminal_failure_a_script_run_cannot_declare_is_reported_as_a_defect() {
    let identity = viewer();
    let payload = json!({ "threadId": "task-1", "scriptId": "dev" });
    let error = ScriptsError::Terminal(terminal::TerminalError::Workspace(
        WorkspaceAccessError::NoWorktree,
    ));

    let request_id: RequestId = serde_json::from_value(json!(1)).expect("a numeric id");
    let exit = scripts_failure_exit(&request_id, &error, &payload, &identity);

    assert_eq!(exit["exit"]["cause"][0]["_tag"], "Die");
}

#[test]
fn gate_parser_accepts_enabled_values() {
    for value in ["1", "true", "TRUE", "yes", "on", " on "] {
        assert!(parse_enabled("T3_UI_RPC_ENABLED", value).unwrap());
    }
}

#[test]
fn gate_parser_accepts_disabled_values() {
    for value in ["0", "false", "FALSE", "no", "off", "", "  "] {
        assert!(!parse_enabled("T3_UI_RPC_ENABLED", value).unwrap());
    }
}

#[test]
fn gate_parser_rejects_invalid_values() {
    let error = parse_enabled("T3_UI_RPC_ENABLED", "sometimes").unwrap_err();

    assert!(matches!(
        error,
        UiRpcConfigError::Invalid(message)
            if message == "T3_UI_RPC_ENABLED must be true or false"
    ));
}

/// A deployment whose Sandboxes are all unreachable.
///
/// Every file, server and log request these tests make is refused before it
/// would reach a Sandbox — by the `cwd`, by a listing that contains no
/// conversations, or by a scoped Task read that finds nothing — so these
/// seams exist to be held, not called. What a reachable Sandbox answers is
/// a gRPC or K8s round trip and belongs in an integration test.
struct UnreachableSandboxes;

fn unreachable(task_id: &str) -> SandboxError {
    SandboxError::NotReady(task_id.to_string())
}

#[async_trait]
impl SandboxConnector for UnreachableSandboxes {
    async fn sandbox_client(
        &self,
        task_id: &str,
    ) -> Result<sandbox_proto::client::SandboxClient, SandboxError> {
        Err(unreachable(task_id))
    }
}

#[async_trait]
impl SandboxIntentReader for UnreachableSandboxes {
    async fn desired_state(
        &self,
        task_id: &str,
    ) -> Result<moatless_types::sandbox::SandboxDesiredState, SandboxError> {
        Err(unreachable(task_id))
    }
}

#[async_trait]
impl SandboxPortPublisher for UnreachableSandboxes {
    async fn publish_port(
        &self,
        task_id: &str,
        _name: &str,
        _port: u16,
    ) -> Result<Option<String>, SandboxError> {
        Err(unreachable(task_id))
    }
}

#[async_trait]
impl SandboxStarter for UnreachableSandboxes {
    async fn start(&self, task_id: &str) -> Result<(), moatless_ports::sandbox::SandboxError> {
        Err(unreachable(task_id))
    }
}

#[async_trait]
impl SandboxStopper for UnreachableSandboxes {
    async fn stop(
        &self,
        task_id: &str,
        _reason: moatless_types::sandbox::SandboxStopReason,
    ) -> Result<(), moatless_ports::sandbox::SandboxError> {
        Err(unreachable(task_id))
    }
}

#[async_trait]
impl moatless_ports::sandbox::SandboxOperator for UnreachableSandboxes {
    async fn restart(&self, task_id: &str) -> Result<(), moatless_ports::sandbox::SandboxError> {
        Err(unreachable(task_id))
    }

    async fn redeploy(&self, task_id: &str) -> Result<(), moatless_ports::sandbox::SandboxError> {
        Err(unreachable(task_id))
    }

    async fn cleanup(&self, task_id: &str) -> Result<(), moatless_ports::sandbox::SandboxError> {
        Err(unreachable(task_id))
    }

    async fn expected_image(
        &self,
        task_id: &str,
    ) -> Result<String, moatless_ports::sandbox::SandboxError> {
        Err(unreachable(task_id))
    }
}

#[async_trait]
impl ClusterDiagnostics for UnreachableSandboxes {
    async fn list_sandbox_diagnostics(
        &self,
    ) -> Result<
        Vec<moatless_ports::sandbox::SandboxDiagnosticsEntry>,
        moatless_ports::sandbox::SandboxError,
    > {
        Ok(Vec::new())
    }
}

#[async_trait]
impl SandboxObserver for UnreachableSandboxes {
    async fn sandbox_status(
        &self,
        task_id: &str,
        _task_status: moatless_types::task::TaskStatus,
        _desired_state: moatless_types::sandbox::SandboxDesiredState,
    ) -> Result<ContractSandboxStatus, SandboxError> {
        Err(unreachable(task_id))
    }

    async fn live_status(
        &self,
        task_id: &str,
        _task_status: moatless_types::task::TaskStatus,
        _desired_state: moatless_types::sandbox::SandboxDesiredState,
    ) -> Result<LiveStatus, SandboxError> {
        Err(unreachable(task_id))
    }

    async fn detail(
        &self,
        task_id: &str,
        _task_status: moatless_types::task::TaskStatus,
        _desired_state: moatless_types::sandbox::SandboxDesiredState,
    ) -> Result<moatless_ports::sandbox::SandboxDetail, SandboxError> {
        Err(unreachable(task_id))
    }

    async fn server_logs(
        &self,
        task_id: &str,
        _server_name: &str,
        _options: LogOptions,
    ) -> Result<ServerLogStream, SandboxError> {
        Err(unreachable(task_id))
    }
}

struct TestPairingCodes(Option<PairingCode>);

#[async_trait]
impl PairingCodeDao for TestPairingCodes {
    async fn insert_pairing_code(&self, _code: &PairingCode) -> Result<(), sqlx::Error> {
        Ok(())
    }

    async fn consume_pairing_code(
        &self,
        code_hash: &str,
        _now: DateTime<Utc>,
    ) -> Result<Option<PairingCode>, sqlx::Error> {
        Ok(self
            .0
            .as_ref()
            .filter(|code| code.code_hash == code_hash)
            .cloned())
    }

    async fn delete_finished_pairing_codes(
        &self,
        _cutoff: DateTime<Utc>,
    ) -> Result<u64, sqlx::Error> {
        Ok(0)
    }
}

struct TestApiKeyIssuer;

#[async_trait]
impl PairingApiKeyIssuer for TestApiKeyIssuer {
    async fn issue_api_key(
        &self,
        _user_id: &str,
        name: &str,
        scopes: Vec<String>,
        expires_at: DateTime<Utc>,
        _group_id: Option<&str>,
    ) -> Result<PairedDevice, PairingError> {
        assert_eq!(name, "T3 Code Mobile (iOS)");
        Ok(PairedDevice {
            key_id: "key-1".into(),
            token: "mvk_test-token".into(),
            expires_at,
            scopes,
        })
    }
}

fn test_pairing(code: Option<PairingCode>) -> DevicePairing {
    DevicePairing::new(Arc::new(TestPairingCodes(code)), Arc::new(TestApiKeyIssuer))
}

fn pairing_code(code: &str) -> PairingCode {
    PairingCode {
        id: "pairing-1".into(),
        code_hash: hash_key(code),
        user_id: Uuid::nil().to_string(),
        group_id: None,
        label: None,
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::minutes(5),
    }
}

/// The full catalog with everything installed. The provider list's own
/// shape is asserted in `providers.rs`; here it only has to be non-empty so
/// the composer is not disabled.
struct StubAgents;

#[async_trait]
impl ports::AgentCatalogSource for StubAgents {
    async fn list(
        &self,
        _viewer: &ScopeContext,
    ) -> Vec<moatless_api_schemas::agent::AgentDescriptor> {
        crate::agent::catalog::catalog()
            .iter()
            .map(|entry| moatless_api_schemas::agent::AgentDescriptor {
                id: entry.id.to_string(),
                label: entry.label.to_string(),
                installed: true,
                default_model: entry.default_model.to_string(),
                models: entry
                    .models
                    .iter()
                    .map(|m| moatless_api_schemas::agent::AgentModel {
                        id: m.id.to_string(),
                        label: m.label.to_string(),
                    })
                    .collect(),
                missing_credential_hint: None,
            })
            .collect()
    }
}

/// The one skill every Workspace offers these tests.
struct StubSkills;

#[async_trait]
impl ports::SkillCatalog for StubSkills {
    async fn list_for_workspace(
        &self,
        _viewer: &ScopeContext,
        _caller: Uuid,
        _workspace_id: &str,
    ) -> Vec<ports::WorkspaceSkill> {
        vec![ports::WorkspaceSkill {
            name: "git-workflow".to_string(),
            description: "Commit and PR flow".to_string(),
            plugin_name: None,
        }]
    }
}

/// No test in this file dispatches a project-root `vcs.listRefs`, so the
/// git-host branch listing is never reached here; it is asserted in
/// `review.rs` against a mock workspace DAO and branch source.
struct UnreachableBranches;

#[async_trait]
impl ports::RepositoryBranchSource for UnreachableBranches {
    async fn list_branches(
        &self,
        _viewer: &ScopeContext,
        _repository_id: &str,
        _workspace_id: Option<&str>,
    ) -> Result<moatless_api_schemas::repository::BranchListResponse, ports::BranchListError> {
        unreachable!("no test in this file lists a project's branches")
    }
}

/// A Task in these envelope tests is never bound to a pull request; what a
/// bound one projects to is asserted in `review.rs`.
struct NoBoundPullRequests;

#[async_trait]
impl ports::BoundPullRequestSource for NoBoundPullRequests {
    async fn bound_pull_requests(&self, _task_id: &str) -> Vec<ports::BoundPullRequest> {
        Vec::new()
    }
    async fn tasks_bound_to(&self, _repository: &str, _number: u64) -> Vec<String> {
        Vec::new()
    }
    async fn resolve_pull_request(
        &self,
        _viewer: &ScopeContext,
        _repository_id: &str,
        _reference: &str,
    ) -> Result<ports::ResolvedPullRequest, ports::PullRequestError> {
        unreachable!("no test in this file resolves a pull request")
    }
    async fn open_pull_request_for_head(
        &self,
        _viewer: &ScopeContext,
        _repository_id: &str,
        _head: &str,
    ) -> Option<ports::ResolvedPullRequest> {
        None
    }
}

/// The envelope tests never sign in with GitHub; what an authenticated badge
/// projects to is asserted in `git_actions.rs`.
struct NoGithubIdentity;

#[async_trait]
impl ports::SourceControlAuthSource for NoGithubIdentity {
    async fn github_identity(&self, _viewer: &ScopeContext) -> Option<ports::GithubIdentity> {
        None
    }
}

struct NoChildTasks;

#[async_trait]
impl ports::TaskRelationSource for NoChildTasks {
    async fn children(
        &self,
        _parent_task_id: &str,
    ) -> Result<Vec<ports::ChildTask>, moatless_types::store::StoreError> {
        Ok(Vec::new())
    }
}

/// What reached the attachment store, so the mint, the upload route and
/// `attachments.delete` can be asserted without a database or an S3 bucket
/// behind them.
#[derive(Default)]
struct RecordingAttachments {
    /// Each stored attachment as `(id, filename, mime type, byte count)`.
    stored: std::sync::Mutex<Vec<(String, String, String, usize)>>,
    deleted: std::sync::Mutex<Vec<String>>,
    refuse: Option<ports::AttachmentStoreError>,
}

impl RecordingAttachments {
    fn refusal(&self) -> Result<(), ports::AttachmentStoreError> {
        match &self.refuse {
            None => Ok(()),
            Some(ports::AttachmentStoreError::Forbidden) => {
                Err(ports::AttachmentStoreError::Forbidden)
            }
            Some(ports::AttachmentStoreError::Failed(message)) => {
                Err(ports::AttachmentStoreError::Failed(message.clone()))
            }
        }
    }
}

#[async_trait]
impl ports::AttachmentStore for RecordingAttachments {
    async fn store(
        &self,
        _viewer: &ScopeContext,
        attachment_id: &str,
        declared: ports::DeclaredAttachment,
        bytes: Vec<u8>,
    ) -> Result<(), ports::AttachmentStoreError> {
        #[allow(clippy::unwrap_used)]
        self.stored.lock().unwrap().push((
            attachment_id.to_string(),
            declared.filename,
            declared.mime_type,
            bytes.len(),
        ));
        self.refusal()
    }

    async fn delete(
        &self,
        _viewer: &ScopeContext,
        attachment_id: &str,
    ) -> Result<(), ports::AttachmentStoreError> {
        #[allow(clippy::unwrap_used)]
        self.deleted.lock().unwrap().push(attachment_id.to_string());
        self.refusal()
    }
}

/// Nothing in this file dispatches a command that reaches a write; the
/// command table itself is asserted in `commands.rs` against a recording
/// sink.
struct UnreachableUsage;

#[async_trait]
impl ports::UsageReporter for UnreachableUsage {
    async fn buckets(
        &self,
        _viewer: &ScopeContext,
        _window: &ports::UsageWindow,
    ) -> Result<Vec<ports::UsageBucketRow>, moatless_types::store::StoreError> {
        unreachable!("no test in this file reads usage")
    }
}

struct UnreachableCommands;

#[async_trait]
impl ports::ThreadCommandSink for UnreachableCommands {
    async fn send_turn(
        &self,
        _viewer: &ScopeContext,
        _turn: ports::ThreadTurn,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file sends a turn")
    }
    async fn create_thread(
        &self,
        _viewer: &ScopeContext,
        _thread: ports::NewThread,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file creates a thread")
    }
    async fn update_meta(
        &self,
        _viewer: &ScopeContext,
        _meta: ports::ThreadMeta,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file updates metadata")
    }
    async fn interrupt(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file interrupts a turn")
    }
    async fn stop(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file stops a thread")
    }
    async fn set_idle_timeout(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
        _minutes: i32,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file changes an idle timeout")
    }
    async fn archive(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file archives a thread")
    }
    async fn unarchive(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file unarchives a thread")
    }
    async fn settle(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file settles a thread")
    }
    async fn unsettle(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file unsettles a thread")
    }
    async fn snooze(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
        _snoozed_until: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file snoozes a thread")
    }
    async fn unsnooze(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file unsnoozes a thread")
    }
    async fn answer_question(
        &self,
        _viewer: &ScopeContext,
        _answer: ports::QuestionAnswer,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file answers a question")
    }
    async fn fork(
        &self,
        _viewer: &ScopeContext,
        _fork: ports::ForkThread,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file forks a thread")
    }
}

/// These tests are about the envelope encoding, so the listing behind it
/// is a stub. What a viewer actually sees is asserted against a real
/// database in `backend/tests/integration_surface/ui_rpc_shell_test.rs`.
fn state() -> UiRpcState {
    state_with_attachments(Arc::new(RecordingAttachments::default()))
}

fn state_with_attachments(attachments: Arc<dyn ports::AttachmentStore>) -> UiRpcState {
    let mut workspaces = MockT3WorkspaceDao::new();
    workspaces
        .expect_list_readable_including()
        .returning(|_, _| Ok(Vec::new()));
    let mut conversations = MockConversationDao::new();
    conversations
        .expect_list_followed()
        .returning(|_| Ok(Vec::new()));
    conversations
        .expect_list_followed_by_ids()
        .returning(|_, _| Ok(Vec::new()));
    conversations
        .expect_readable_by_id()
        .returning(|_, _| Ok(None));
    conversations.expect_listing_head().returning(|| {
        Ok(UiRpcListingHead {
            identity: stub_identity(),
            position: 0,
        })
    });
    conversations
        .expect_listing_changes_after()
        .returning(|_, _| Ok(Vec::new()));
    // The stub holds no conversations, so every one asked for is one that
    // is gone: nothing has ever changed and nothing is named. What a
    // conversation that exists projects to is asserted in `projection.rs`.
    conversations
        .expect_conversation_head()
        .returning(|_| Ok(ConversationHead { position: 0 }));
    conversations
        .expect_conversation_changes()
        .returning(|_, _, _| Ok(Vec::new()));
    conversations
        .expect_readable_by_ids()
        .returning(|_, _| Ok(Vec::new()));
    let conversations: Arc<dyn ConversationDao> = Arc::new(conversations);
    UiRpcState {
        rpc: Arc::new(UiRpcConfig {
            environment_id: environment_id(&stub_identity()),
            allowed_origins: vec![AllowedOrigin::parse("http://localhost:5733")],
            otlp_traces_enabled: false,
            attachment_uploads: true,
            usage_limit_sources: false,
        }),
        environment: Arc::new(stub_identity()),
        session_cookie_name: Arc::from("moatless_session"),
        frontend_url: Arc::from("http://localhost:3000"),
        session_jwt_secret: Some(Arc::from("test-secret")),
        pairing: Arc::new(test_pairing(None)),
        workspaces: Arc::new(workspaces),
        workspace_script_commands: Arc::new(ports::MockWorkspaceScriptCommands::new()),
        conversations: conversations.clone(),
        sandboxes: Arc::new(UnreachableSandboxes),
        snapshots: None,
        servers: ServersState {
            observer: Arc::new(UnreachableSandboxes),
            tasks: Arc::new(MockT3TaskDao::new()),
            intent: Arc::new(UnreachableSandboxes),
        },
        scripts: ScriptsState {
            workspaces: Arc::new(MockT3WorkspaceDao::new()),
            ports: Arc::new(UnreachableSandboxes),
            preview: PreviewState {
                tabs: Arc::new(preview_tabs::InMemoryPreviewTabDao::new()),
                tasks: Arc::new(MockT3TaskDao::new()),
            },
        },
        sandbox: SandboxState {
            observer: Arc::new(UnreachableSandboxes),
            tasks: Arc::new(MockT3TaskDao::new()),
            intent: Arc::new(UnreachableSandboxes),
            starter: Arc::new(UnreachableSandboxes),
            stopper: Arc::new(UnreachableSandboxes),
            operator: Arc::new(UnreachableSandboxes),
            diagnostics: Arc::new(UnreachableSandboxes),
        },
        subtasks: SubtasksState {
            relations: Arc::new(NoChildTasks),
            conversations,
        },
        preview: PreviewState {
            tabs: Arc::new(preview_tabs::InMemoryPreviewTabDao::new()),
            tasks: Arc::new(MockT3TaskDao::new()),
        },
        agents: Arc::new(StubAgents),
        branches: Arc::new(UnreachableBranches),
        skills: Arc::new(StubSkills),
        pull_requests: Arc::new(NoBoundPullRequests),
        source_control_auth: Arc::new(NoGithubIdentity),
        commands: Arc::new(UnreachableCommands),
        attachments,
        usage: Arc::new(UnreachableUsage),
        model_defaults: Arc::new(AgentModelDefaults::default()),
        terminal: terminal::TerminalHub::default(),
        client_traces: observability::ClientTraceForwarder::default(),
        usage_limit_sources: crate::usage_limit_sources::UsageLimitSources::disabled(),
    }
}

/// A session whose pushed frames go nowhere. These tests drive the request
/// side; what a subscription's poller produces is asserted against a real
/// database in `backend/tests/integration_surface/ui_rpc_shell_test.rs`.
///
/// A detached request answers on that channel rather than to its caller, so a
/// test naming one of the tags `rpc::dispatch::detach` matches uses
/// [`connection`] instead.
fn session() -> ProtocolSession {
    ProtocolSession::new(mpsc::channel(OUTBOUND_FRAMES).0)
}

/// A session holding the frame channel `serve_websocket` writes to the socket.
struct Connection {
    session: ProtocolSession,
    pushed: mpsc::Receiver<Value>,
}

fn connection() -> Connection {
    let (frames, pushed) = mpsc::channel(OUTBOUND_FRAMES);
    Connection {
        session: ProtocolSession::new(frames),
        pushed,
    }
}

impl Connection {
    /// Answer one client frame and collect everything it put on the wire: what
    /// the frame loop returned, then what a detached request pushed, in the
    /// order `serve_websocket` would have written them.
    async fn exchange(
        &mut self,
        bytes: &[u8],
        state: &UiRpcState,
        identity: &UiRpcAuthIdentity,
    ) -> ProtocolOutput {
        let mut output = self.session.handle_json(bytes, state, identity).await;
        self.session.detached.wait_until_idle().await;
        while let Some(frame) = self.next_pushed() {
            output.frames.push(frame);
        }
        output
    }

    fn next_pushed(&mut self) -> Option<Value> {
        self.pushed.try_recv().ok()
    }
}

fn viewer() -> UiRpcAuthIdentity {
    UiRpcAuthIdentity {
        user_id: Uuid::nil(),
        login: "viewer".into(),
        role: UserRole::User,
        provider: "test".into(),
        group_id: None,
    }
}

fn request(id: &str, tag: &str, payload: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "_tag": "Request",
        "id": id,
        "tag": tag,
        "payload": payload,
        "headers": []
    }))
    .unwrap()
}

#[tokio::test]
async fn a_request_id_returns_in_the_json_type_it_arrived_as() {
    // The client resolves a response by looking `requestId` up in a map
    // keyed by the value it sent, and coerces nothing. Answering `21` with
    // `"21"` matches no caller, so the request never completes and the
    // connection attempt times out with every frame looking correct.
    for id in [json!(21), json!("21")] {
        let frame = serde_json::to_vec(&json!({
            "_tag": "Request",
            "id": id,
            "tag": "server.getConfig",
            "payload": {},
            "headers": []
        }))
        .unwrap();
        let mut connection = connection();
        let output = connection.exchange(&frame, &state(), &viewer()).await;
        assert_eq!(output.frames[0]["_tag"], "Exit");
        assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
        assert_eq!(
            output.frames[0]["requestId"], id,
            "a {id} request id must come back as {id}, not re-typed"
        );
    }
}

#[tokio::test]
async fn a_numeric_stream_id_returns_numeric_on_every_frame() {
    let frame = serde_json::to_vec(&json!({
        "_tag": "Request",
        "id": 4,
        "tag": "subscribeServerConfig",
        "payload": {},
        "headers": []
    }))
    .unwrap();
    let mut session = session();
    let output = session.handle_json(&frame, &state(), &viewer()).await;
    assert_eq!(output.frames[0]["_tag"], "Chunk");
    assert_eq!(
        output.frames[0]["requestId"],
        json!(4),
        "a stream's chunks correlate by the same id its request carried"
    );

    let interrupt = serde_json::to_vec(&json!({ "_tag": "Interrupt", "requestId": 4 })).unwrap();
    let output = session.handle_json(&interrupt, &state(), &viewer()).await;
    assert_eq!(output.frames[0]["_tag"], "Exit");
    assert_eq!(output.frames[0]["requestId"], json!(4));
}

/// A fleet with one Sandbox in it, for the dispatch test below. Real
/// mapping from pod list and metrics into this shape is unit-tested
/// against fixtures in `sandbox::k8s::pod_reader`; this only checks that
/// the RPC arm serializes what the seam hands it, unchanged.
struct FakeFleet;

#[async_trait]
impl ClusterDiagnostics for FakeFleet {
    async fn list_sandbox_diagnostics(&self) -> Result<Vec<SandboxDiagnosticsEntry>, SandboxError> {
        Ok(vec![SandboxDiagnosticsEntry {
            task_id: "task-1".into(),
            title: None,
            sandbox_status: SandboxStatus::Ready,
            node_name: Some("node-a".into()),
            containers: vec![SandboxDiagnosticsContainer {
                name: "sandbox".into(),
                state: "running".into(),
                restart_count: 0,
                cpu_millicores: Some(100),
                memory_mb: Some(256),
            }],
            total_cpu_millicores: Some(100),
            total_memory_mb: Some(256),
        }])
    }
}

#[tokio::test]
async fn a_sandbox_list_diagnostics_request_returns_the_mapped_fleet() {
    let mut state = state();
    state.sandbox.diagnostics = Arc::new(FakeFleet);
    let frame = request("1", "sandbox.listDiagnostics", json!({}));
    let mut connection = connection();
    let output = connection.exchange(&frame, &state, &viewer()).await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    let sandboxes = &output.frames[0]["exit"]["value"]["sandboxes"];
    assert_eq!(sandboxes[0]["taskId"], "task-1");
    assert_eq!(sandboxes[0]["nodeName"], "node-a");
    assert_eq!(sandboxes[0]["containers"][0]["name"], "sandbox");
    assert_eq!(sandboxes[0]["containers"][0]["cpuMillicores"], 100);
    assert_eq!(sandboxes[0]["totalMemoryMb"], 256);
}

/// A fleet read that answers only once the test releases it, for the ordering
/// test below.
struct BlockedFleet(Arc<Notify>);

#[async_trait]
impl ClusterDiagnostics for BlockedFleet {
    async fn list_sandbox_diagnostics(&self) -> Result<Vec<SandboxDiagnosticsEntry>, SandboxError> {
        self.0.notified().await;
        Ok(Vec::new())
    }
}

/// The whole point of detaching: a slow read holds only itself. Before it, one
/// connection answered one request at a time, so everything a client had sent
/// sat unread on the socket behind the slowest handler in the batch.
// Paused time, so the deadline below costs nothing to reach: an inline read
// would sit on `BlockedFleet` forever, and a hung test says less than a failed
// one.
#[tokio::test(start_paused = true)]
async fn a_slow_read_does_not_hold_the_next_request_behind_it() {
    let released = Arc::new(Notify::new());
    let mut state = state();
    state.sandbox.diagnostics = Arc::new(BlockedFleet(released.clone()));
    let mut connection = connection();

    let slow = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        connection.session.handle_json(
            &request("1", "sandbox.listDiagnostics", json!({})),
            &state,
            &viewer(),
        ),
    )
    .await
    .expect("the frame loop is not waiting on the read");
    assert!(
        slow.frames.is_empty(),
        "the read has not answered, and the loop did not wait for it"
    );

    let fast = connection
        .session
        .handle_json(&request("2", "server.probe", json!({})), &state, &viewer())
        .await;
    assert_eq!(
        fast.frames[0]["requestId"], "2",
        "answered while the read is still running"
    );

    released.notify_one();
    connection.session.detached.wait_until_idle().await;
    let answered = connection
        .next_pushed()
        .expect("the read answers in the end");
    assert_eq!(answered["requestId"], "1");
    assert_eq!(answered["exit"]["_tag"], "Success");
}

/// An observer that reports one fixed status, for the subscribe test below.
struct FixedStatus(SandboxStatus);

#[async_trait]
impl SandboxIntentReader for FixedStatus {
    async fn desired_state(&self, _task_id: &str) -> Result<SandboxDesiredState, SandboxError> {
        Ok(SandboxDesiredState::Running)
    }
}

#[async_trait]
impl SandboxObserver for FixedStatus {
    async fn sandbox_status(
        &self,
        _task_id: &str,
        _task_status: TaskStatus,
        _desired_state: SandboxDesiredState,
    ) -> Result<ContractSandboxStatus, SandboxError> {
        unreachable!("the lifecycle read uses live_status")
    }

    async fn live_status(
        &self,
        _task_id: &str,
        _task_status: TaskStatus,
        _desired_state: SandboxDesiredState,
    ) -> Result<LiveStatus, SandboxError> {
        Ok(LiveStatus {
            sandbox_status: self.0,
            agent_status: None,
            sandbox_error: None,
            commands: Vec::new(),
        })
    }

    async fn detail(
        &self,
        _task_id: &str,
        _task_status: TaskStatus,
        _desired_state: SandboxDesiredState,
    ) -> Result<moatless_ports::sandbox::SandboxDetail, SandboxError> {
        unreachable!("the lifecycle read reads no detail")
    }

    async fn server_logs(
        &self,
        _task_id: &str,
        _server_name: &str,
        _options: LogOptions,
    ) -> Result<ServerLogStream, SandboxError> {
        unreachable!("the lifecycle read reads no logs")
    }
}

fn state_with_sandbox_status(status: SandboxStatus) -> UiRpcState {
    let mut state = state();
    let mut tasks = MockT3TaskDao::new();
    tasks.expect_find_by_id().returning(|_, _| {
        Ok(Some(TaskView {
            status: TaskStatus::Open,
            ttl_minutes: 15,
        }))
    });
    state.sandbox.tasks = Arc::new(tasks);
    state.sandbox.observer = Arc::new(FixedStatus(status));
    state.sandbox.intent = Arc::new(FixedStatus(status));
    state
}

/// The subscription seeds before its first poll interval elapses.
///
/// Without the seed the indicator would render nothing for the first
/// interval on every mount — which is the stale-status bug the push channel
/// exists to end, just moved to the start of the stream.
#[tokio::test]
async fn subscribing_to_a_sandbox_status_answers_before_the_first_poll() {
    let frame = request(
        "1",
        "sandbox.subscribeStatus",
        json!({ "threadId": "task-1" }),
    );
    let mut session = session();
    let output = session
        .handle_json(
            &frame,
            &state_with_sandbox_status(SandboxStatus::Ready),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["_tag"], "Chunk");
    assert_eq!(output.frames[0]["values"][0]["sandboxStatus"], "ready");
}

/// A subscription is per-thread, so the thread is not optional. Left to the
/// poller this would be a stream that authorizes nothing and never emits.
#[tokio::test]
async fn subscribing_to_a_sandbox_status_without_a_thread_is_refused() {
    let frame = request("1", "sandbox.subscribeStatus", json!({}));
    let mut session = session();
    let output = session.handle_json(&frame, &state(), &viewer()).await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
    assert!(
        output.frames[0]["exit"]["cause"][0]["defect"]
            .as_str()
            .unwrap_or_default()
            .contains("threadId"),
        "the refusal should name what is missing, got {output:?}"
    );
}

/// The capability is how a client knows it may stop polling `sandbox.status`.
/// A server that serves the subscription but stays quiet about it leaves
/// every client on the poll.
#[tokio::test]
async fn the_descriptor_advertises_the_sandbox_status_push() {
    let frame = request("1", "server.getConfig", json!({}));
    let mut connection = connection();
    let output = connection.exchange(&frame, &state(), &viewer()).await;

    assert_eq!(
        output.frames[0]["exit"]["value"]["environment"]["capabilities"]["sandboxStatusPush"],
        json!(true)
    );
}

/// `agentStatus` is `optionalKey` and absent whenever the sandbox is not
/// up, so its absence from one answer says nothing about the server. The
/// capability is the only thing that distinguishes a server that reports it
/// from one that never will.
#[tokio::test]
async fn the_descriptor_advertises_the_sandbox_agent_status() {
    let frame = request("1", "server.getConfig", json!({}));
    let mut connection = connection();
    let output = connection.exchange(&frame, &state(), &viewer()).await;

    assert_eq!(
        output.frames[0]["exit"]["value"]["environment"]["capabilities"]["sandboxAgentStatus"],
        json!(true)
    );
}

/// `commands` is omitted when none are registered, which is most of the
/// time, so its absence from one answer says nothing about the server
/// either. Same reasoning as `sandboxAgentStatus` above.
#[tokio::test]
async fn the_descriptor_advertises_the_sandbox_commands() {
    let frame = request("1", "server.getConfig", json!({}));
    let mut connection = connection();
    let output = connection.exchange(&frame, &state(), &viewer()).await;

    assert_eq!(
        output.frames[0]["exit"]["value"]["environment"]["capabilities"]["sandboxCommands"],
        json!(true)
    );
}

/// The two keys the composer reads before it decides how to send a file.
/// Reporting `attachmentUploads` moves it off the inline data-URL path for
/// images too, so a server that reports one without serving the other
/// loses the person their attachment.
#[tokio::test]
async fn the_descriptor_advertises_both_attachment_capabilities_together() {
    let frame = request("1", "server.getConfig", json!({}));
    let mut connection = connection();
    let output = connection.exchange(&frame, &state(), &viewer()).await;
    let capabilities = &output.frames[0]["exit"]["value"]["environment"]["capabilities"];

    assert_eq!(capabilities["attachmentUploads"], json!(true));
    assert_eq!(
        capabilities["fileAttachments"],
        json!({ "maxUploadBytes": 50 * 1024 * 1024 })
    );
}

/// Both keys are `optionalKey` in the contract, so a deployment that cannot
/// sign an upload URL has to omit them rather than answer `false` or
/// `null`: the client decodes the whole descriptor before it can connect.
#[test]
fn a_deployment_that_cannot_sign_an_upload_url_omits_both_keys() {
    let config = UiRpcConfig {
        environment_id: "moatless".into(),
        allowed_origins: vec![AllowedOrigin::parse("https://example.com")],
        otlp_traces_enabled: false,
        attachment_uploads: false,
        usage_limit_sources: false,
    };
    let capabilities = &descriptor(&config)["capabilities"];

    assert!(capabilities.get("attachmentUploads").is_none());
    assert!(capabilities.get("fileAttachments").is_none());
    assert!(capabilities.get("usageLimitSources").is_none());
}

/// The client drops `usageLimitSourcesUpdated` unless the descriptor declares
/// the capability.
#[test]
fn a_deployment_with_a_hub_declares_the_usage_limit_source_capability() {
    let config = UiRpcConfig {
        environment_id: "moatless".into(),
        allowed_origins: vec![AllowedOrigin::parse("https://example.com")],
        otlp_traces_enabled: false,
        attachment_uploads: false,
        usage_limit_sources: true,
    };

    assert_eq!(
        descriptor(&config)["capabilities"]["usageLimitSources"],
        true
    );
}

/// What the composer gets back before it moves any bytes: the id it will
/// name on the turn, and the URL it will `POST` to.
#[tokio::test]
async fn create_upload_url_answers_an_id_and_a_url_under_the_upload_prefix() {
    let frame = request(
        "1",
        "attachments.createUploadUrl",
        json!({ "type": "file", "name": "report.pdf", "mimeType": "application/pdf", "sizeBytes": 2048 }),
    );
    let mut session = session();
    let output = session.handle_json(&frame, &state(), &viewer()).await;
    let value = &output.frames[0]["exit"]["value"];

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert!(
        value["attachmentId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    assert!(
        value["relativeUrl"]
            .as_str()
            .is_some_and(|url| url.starts_with("/api/attachments/")),
        "{value}"
    );
    assert!(value["expiresAt"].as_i64().is_some_and(|at| at > 0));
}

/// A payload outside the union never becomes an upload URL. The client's
/// own schema refuses these before they are sent, so this is a malformed
/// request rather than one of the method's declared outcomes.
#[tokio::test]
async fn create_upload_url_refuses_a_payload_outside_the_union() {
    let frame = request(
        "1",
        "attachments.createUploadUrl",
        json!({ "type": "image", "name": "a.tiff", "mimeType": "image/tiff", "sizeBytes": 10 }),
    );
    let mut session = session();
    let output = session.handle_json(&frame, &state(), &viewer()).await;

    assert_eq!(output.frames[0]["exit"]["cause"][0]["_tag"], "Die");
}

/// The composer deletes an attachment it abandoned — a cancel, a retry, a
/// discarded draft — without knowing whether the upload ever landed, so
/// this succeeds either way and the id is what has to reach the store.
#[tokio::test]
async fn delete_reaches_the_store_with_the_id_it_was_given() {
    let attachments = Arc::new(RecordingAttachments::default());
    let frame = request(
        "1",
        "attachments.delete",
        json!({ "attachmentId": "6b1f7b2e-0000-4000-8000-000000000001" }),
    );
    let mut session = session();
    let output = session
        .handle_json(
            &frame,
            &state_with_attachments(attachments.clone()),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    #[allow(clippy::unwrap_used)]
    let deleted = attachments.deleted.lock().unwrap().clone();
    assert_eq!(deleted, vec!["6b1f7b2e-0000-4000-8000-000000000001"]);
}

/// Somebody else's attachment is, from here, one that is not there:
/// answering differently would make this a way to ask whether an id exists.
#[tokio::test]
async fn delete_of_an_attachment_this_viewer_may_not_touch_still_succeeds() {
    let attachments = Arc::new(RecordingAttachments {
        refuse: Some(ports::AttachmentStoreError::Forbidden),
        ..RecordingAttachments::default()
    });
    let frame = request("1", "attachments.delete", json!({ "attachmentId": "abc" }));
    let mut session = session();
    let output = session
        .handle_json(&frame, &state_with_attachments(attachments), &viewer())
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
}

/// An id that could leave the route it becomes a segment of never reaches
/// the store.
#[tokio::test]
async fn delete_of_something_that_is_not_an_id_never_reaches_the_store() {
    let attachments = Arc::new(RecordingAttachments::default());
    let frame = request(
        "1",
        "attachments.delete",
        json!({ "attachmentId": "../secrets" }),
    );
    let mut session = session();
    let output = session
        .handle_json(
            &frame,
            &state_with_attachments(attachments.clone()),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["cause"][0]["_tag"], "Die");
    #[allow(clippy::unwrap_used)]
    let deleted = attachments.deleted.lock().unwrap().len();
    assert_eq!(deleted, 0);
}

#[tokio::test]
async fn a_numeric_id_is_accepted_on_every_frame_that_carries_one() {
    for frame in [
        json!({ "_tag": "Ack", "requestId": 7 }),
        json!({ "_tag": "Interrupt", "requestId": 7 }),
    ] {
        let tag = frame["_tag"].as_str().unwrap().to_string();
        let mut session = session();
        let output = session
            .handle_json(&serde_json::to_vec(&frame).unwrap(), &state(), &viewer())
            .await;
        let defected = output.frames.iter().any(|frame| frame["_tag"] == "Defect");
        assert!(
            !defected,
            "{tag} with a numeric id should parse, got {output:?}"
        );
    }
}

#[tokio::test]
async fn a_fractional_request_id_is_refused() {
    let frame = serde_json::to_vec(&json!({
        "_tag": "Request",
        "id": 21.5,
        "tag": "server.getConfig",
        "payload": {},
        "headers": []
    }))
    .unwrap();
    let mut session = session();
    let output = session.handle_json(&frame, &state(), &viewer()).await;
    assert_eq!(output.frames[0]["_tag"], "Defect");
}

#[tokio::test]
async fn ping_returns_effect_rpc_pong() {
    let mut session = session();
    let output = session
        .handle_json(br#"{"_tag":"Ping"}"#, &state(), &viewer())
        .await;
    assert_eq!(output.frames, vec![json!({ "_tag": "Pong" })]);
}

#[tokio::test]
async fn get_config_returns_minimal_decodable_wire_shape() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request("1", "server.getConfig", json!({})),
            &state(),
            &viewer(),
        )
        .await;
    assert_eq!(output.frames[0]["_tag"], "Exit");
    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(
        output.frames[0]["exit"]["value"]["environment"]["environmentId"],
        environment_id(&stub_identity()),
        "the reported environment id is the configured name and the \
         database's digest"
    );
    assert_eq!(
        output.frames[0]["exit"]["value"]["auth"]["sessionCookieName"],
        "moatless_session"
    );
    assert_eq!(
        output.frames[0]["exit"]["value"]["auth"]["policy"],
        "remote-reachable"
    );
    assert_eq!(
        output.frames[0]["exit"]["value"]["auth"]["bootstrapMethods"],
        json!(["one-time-token"]),
        "a client reads here that it may pair with a code"
    );
    assert_eq!(
        output.frames[0]["exit"]["value"]["auth"]["sessionMethods"],
        json!(["browser-session-cookie", "bearer-access-token"])
    );
    assert_eq!(
        output.frames[0]["exit"]["value"]["threadResumeCompletionMarker"], true,
        "do not advertise this unless subscribeThread can emit synchronized"
    );
}

/// The client polls this for every conversation it holds, most of which
/// have no Sandbox running to ask.
#[tokio::test]
async fn list_refs_answers_a_conversation_with_no_sandbox_rather_than_failing() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request(
                "98",
                "vcs.listRefs",
                json!({
                    "cwd": "/moatless/tasks/task-a",
                    "query": "main",
                    "limit": 10
                }),
            ),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["_tag"], "Exit");
    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(
        output.frames[0]["exit"]["value"],
        json!({
            "refs": [],
            "isRepo": false,
            "hasPrimaryRemote": false,
            "nextCursor": null,
            "totalCount": 0
        })
    );
}

/// Picking a branch for a draft thread is answered as done, echoing the
/// ref, so the client records it and sends it when the Task is created —
/// there is no checkout to switch yet.
#[tokio::test]
async fn switching_a_draft_threads_branch_echoes_the_ref() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "97",
                "vcs.switchRef",
                json!({ "cwd": "/moatless/workspaces/ws-a", "refName": "feature" }),
            ),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(
        output.frames[0]["exit"]["value"],
        json!({ "refName": "feature" })
    );
}

/// A thread that already has a Sandbox cannot change branch — that is fixed
/// at creation — so a switch addressed to a worktree is refused rather than
/// silently claimed as done.
#[tokio::test]
async fn switching_an_existing_threads_branch_is_unsupported() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "96",
                "vcs.switchRef",
                json!({ "cwd": "/moatless/tasks/task-a", "refName": "feature" }),
            ),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
    let cause = &output.frames[0]["exit"]["cause"][0];
    assert_eq!(cause["error"]["_tag"], "UnsupportedMethodError");
}

/// The diff panel is a surface the person opened on purpose, so a
/// conversation it cannot read says why — in the error type the method
/// declares, which is what the panel renders instead of a defect.
#[tokio::test]
async fn a_refused_diff_preview_fails_with_the_method_s_own_error() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request(
                "99",
                "review.getDiffPreview",
                json!({ "cwd": "/moatless/tasks/task-a", "ignoreWhitespace": true }),
            ),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
    let cause = &output.frames[0]["exit"]["cause"][0]["error"];
    assert_eq!(cause["_tag"], "GitCommandError");
    assert_eq!(cause["operation"], "review.getDiffPreview");
    assert_eq!(
        cause["detail"], "This conversation's workspace is not available.",
        "a cwd the viewer's listing does not hold must not report whether the conversation exists"
    );
}

/// Opened for every conversation the client shows, including the ones this
/// surface has no path to. A stream that says nothing is the answer there;
/// a failure would be one the client keeps.
#[tokio::test]
async fn a_vcs_status_subscription_without_an_addressable_cwd_simply_stays_quiet() {
    let mut session = session();
    let output = session
        .handle_json(
            &request("100", "subscribeVcsStatus", json!({ "cwd": "   " })),
            &state(),
            &viewer(),
        )
        .await;

    assert!(output.frames.is_empty(), "no chunk, and no failure either");
}

/// The client calls this one on its own schedule and when a worktree is
/// cleaned up, so an unanswerable `cwd` has to come back as the method's
/// error rather than the defect an unimplemented method produces.
#[tokio::test]
async fn a_status_refresh_answers_rather_than_dying() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request(
                "101",
                "vcs.refreshStatus",
                json!({ "cwd": "/moatless/tasks/task-a" }),
            ),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
    let cause = &output.frames[0]["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail", "a typed refusal, not a Die defect");
    assert_eq!(cause["error"]["_tag"], "GitCommandError");
    assert_eq!(cause["error"]["operation"], "vcs.refreshStatus");
}

/// The composer is disabled while the provider list is empty
/// (`ChatView.logic.ts` has nothing to resolve a model against), so this is
/// the frame that decides whether anybody can type at all.
#[tokio::test]
async fn get_config_publishes_one_provider_per_agent_kind() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request("1", "server.getConfig", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    let providers = output.frames[0]["exit"]["value"]["providers"]
        .as_array()
        .expect("the config carries a provider list");
    assert_eq!(
        providers
            .iter()
            .map(|provider| provider["instanceId"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["claudeAgent", "codex"]
    );
    for provider in providers {
        assert!(
            !provider["models"].as_array().unwrap().is_empty(),
            "{} published no models, so its composer resolves nothing",
            provider["instanceId"]
        );
    }
}

/// T3 calls `refreshProviders` when a person opens the picker, and merges
/// the answer into the same state the config populated. Two lists that
/// disagree would move a thread's provider under it.
#[tokio::test]
async fn refreshing_providers_returns_what_the_config_already_published() {
    let mut connection = connection();
    let state = state();
    let config = connection
        .exchange(
            &request("1", "server.getConfig", json!({})),
            &state,
            &viewer(),
        )
        .await;
    let refreshed = connection
        .exchange(
            &request("2", "server.refreshProviders", json!({})),
            &state,
            &viewer(),
        )
        .await;

    let published = &config.frames[0]["exit"]["value"]["providers"];
    let refreshed = &refreshed.frames[0]["exit"]["value"]["providers"];
    // `checkedAt` is stamped per call and is the one field allowed to
    // differ; everything a client routes on has to match.
    let strip = |list: &Value| -> Vec<Value> {
        list.as_array()
            .unwrap()
            .iter()
            .map(|provider| {
                let mut provider = provider.clone();
                provider.as_object_mut().unwrap().remove("checkedAt");
                provider
            })
            .collect()
    };
    assert_eq!(strip(published), strip(refreshed));
}

/// The composer refreshes providers with the `cwd` it is about to send from,
/// and reads the skills for its `/` menu off the snapshot that comes back.
#[tokio::test]
async fn refreshing_providers_for_a_cwd_publishes_that_workspace_s_skills() {
    let mut connection = connection();
    let refreshed = connection
        .exchange(
            &request(
                "1",
                "server.refreshProviders",
                json!({ "cwd": "/moatless/workspaces/ws-1" }),
            ),
            &state(),
            &viewer(),
        )
        .await;

    let snapshot = &refreshed.frames[0]["exit"]["value"]["providers"][0]["workspaceSnapshots"][0];
    assert_eq!(snapshot["cwd"], "/moatless/workspaces/ws-1");
    assert_eq!(snapshot["skills"][0]["name"], "git-workflow");
    assert_eq!(snapshot["skills"][0]["description"], "Commit and PR flow");
    assert_eq!(snapshot["skills"][0]["scope"], "repo");
}

/// `usageLimitSources` is carried beside the config snapshot, so an opted-in
/// client sees the current hub quota as soon as it connects.
#[tokio::test]
async fn an_opted_in_config_subscriber_gets_the_hub_beside_the_snapshot() {
    let mut session = session();
    let mut state = state();
    state.rpc = Arc::new(UiRpcConfig {
        usage_limit_sources: true,
        ..UiRpcConfig::clone(&state.rpc)
    });
    state.usage_limit_sources = crate::usage_limit_sources::UsageLimitSources::fixed(json!([{
        "id": "cliproxy",
        "kind": "cliproxy",
        "label": "CLIProxyAPI",
        "checkedAt": "2026-09-06T12:00:00.000Z",
        "accounts": []
    }]));

    let subscribed = session
        .handle_json(
            &request(
                "1",
                "subscribeServerConfig",
                json!({ "usageLimitSources": true }),
            ),
            &state,
            &viewer(),
        )
        .await;

    let values = &subscribed.frames[0]["values"];
    assert_eq!(values[0]["type"], "snapshot");
    assert_eq!(values[1]["type"], "usageLimitSourcesUpdated");
    assert_eq!(values[1]["payload"]["sources"][0]["label"], "CLIProxyAPI");
}

/// A client that did not opt in decodes the stream against an event union
/// without this member, so one would take the whole connection down.
#[tokio::test]
async fn a_client_that_did_not_opt_in_is_sent_no_hub_event() {
    let mut session = session();
    let mut state = state();
    state.usage_limit_sources =
        crate::usage_limit_sources::UsageLimitSources::fixed(json!([{ "id": "cliproxy" }]));

    let subscribed = session
        .handle_json(
            &request("1", "subscribeServerConfig", json!({})),
            &state,
            &viewer(),
        )
        .await;

    assert_eq!(
        subscribed.frames[0]["values"].as_array().map(Vec::len),
        Some(1)
    );
}

/// The client's provider state is fed by `subscribeServerConfig` alone — the
/// `refreshProviders` reply only tells the composer whether to retry. A
/// snapshot that never reached the stream would leave the menu empty.
#[tokio::test]
async fn a_resolved_cwd_reaches_an_open_config_stream() {
    let mut connection = connection();
    let state = state();
    connection
        .exchange(
            &request("1", "subscribeServerConfig", json!({})),
            &state,
            &viewer(),
        )
        .await;
    let refreshed = connection
        .exchange(
            &request(
                "2",
                "server.refreshProviders",
                json!({ "cwd": "/moatless/workspaces/ws-1" }),
            ),
            &state,
            &viewer(),
        )
        .await;

    let pushed = &refreshed.frames[1];
    assert_eq!(pushed["_tag"], "Chunk");
    assert_eq!(pushed["requestId"], "1");
    assert_eq!(pushed["values"][0]["type"], "providerStatuses");
    assert_eq!(
        pushed["values"][0]["payload"]["providers"][0]["workspaceSnapshots"][0]["skills"][0]["name"],
        "git-workflow"
    );
}

#[tokio::test]
async fn shell_subscription_chunks_snapshot_and_marker_then_waits_for_ack() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "2",
                "orchestration.subscribeShell",
                json!({ "requestCompletionMarker": true }),
            ),
            &state(),
            &viewer(),
        )
        .await;
    assert_eq!(output.frames[0]["_tag"], "Chunk");
    assert_eq!(output.frames[0]["values"][0]["kind"], "snapshot");
    assert_eq!(output.frames[0]["values"][1]["kind"], "synchronized");
    assert_eq!(session.streams.awaiting_ack("2"), Some(true));

    let ack = session
        .handle_json(br#"{"_tag":"Ack","requestId":"2"}"#, &state(), &viewer())
        .await;
    assert!(ack.frames.is_empty());
    assert_eq!(session.streams.awaiting_ack("2"), Some(false));
}

#[test]
fn thread_initial_values_put_synchronization_after_initial_projection() {
    let snapshot = json!({
        "kind": "snapshot",
        "snapshot": {
            "snapshotSequence": 0,
            "thread": {
                "messages": [{ "text": "Fix the build" }],
                "activities": [{ "summary": "Thinking" }]
            }
        }
    });
    let values = thread_initial_values(vec![snapshot.clone()], true);
    assert_eq!(values, vec![snapshot, json!({ "kind": "synchronized" })]);

    let caught_up = thread_initial_values(Vec::new(), true);
    assert_eq!(caught_up, vec![json!({ "kind": "synchronized" })]);

    assert_eq!(
        thread_initial_values(vec![json!({ "kind": "snapshot" })], false),
        vec![json!({ "kind": "snapshot" })],
        "the marker is emitted only when T3 requested it"
    );
}

/// A client that resumes is not sent the listing again, and the completion
/// marker still arrives — so it stops rendering itself as synchronizing.
///
/// The stub's position is 0 and its change feed is empty, which is a client
/// that is up to date: the correct answer is the marker and nothing else.
/// Before this ticket every subscription began with a snapshot, so the
/// first assertion is what changed.
#[tokio::test]
async fn a_resuming_subscription_chunks_no_snapshot() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "5",
                "orchestration.subscribeShell",
                json!({ "afterSequence": 0, "requestCompletionMarker": true }),
            ),
            &state(),
            &viewer(),
        )
        .await;
    assert_eq!(output.frames[0]["_tag"], "Chunk");
    let values = output.frames[0]["values"].as_array().unwrap();
    assert_eq!(
        values.len(),
        1,
        "a caught-up client should be sent the marker alone, got {values:#?}"
    );
    assert_eq!(values[0]["kind"], "synchronized");
}

/// `afterSequence` is a `NonNegativeInt` in the contract, so nothing a
/// conforming client sends can be a negative number or a string. Reading
/// one as absent rebuilds, which is a listing; reading it as a position
/// would resume from a number nothing allocated.
#[test]
fn an_after_sequence_the_contract_refuses_is_read_as_absent() {
    assert_eq!(after_sequence(&json!({ "afterSequence": 7 })), Some(7));
    assert_eq!(after_sequence(&json!({ "afterSequence": 0 })), Some(0));
    assert_eq!(after_sequence(&json!({})), None);
    assert_eq!(after_sequence(&json!({ "afterSequence": null })), None);
    assert_eq!(after_sequence(&json!({ "afterSequence": -1 })), None);
    assert_eq!(after_sequence(&json!({ "afterSequence": "7" })), None);
    assert_eq!(after_sequence(&json!({ "afterSequence": 1.5 })), None);
}

/// A conversation this viewer cannot see ends the subscription rather than
/// opening one that will never produce a frame.
///
/// The client renders a conversation as still loading until a snapshot
/// arrives, and a stream that opens and stays silent is indistinguishable
/// from one that is slow — which is exactly what an unimplemented
/// `subscribeThread` looked like from the browser.
#[tokio::test]
async fn a_thread_subscription_that_cannot_be_opened_fails_rather_than_hangs() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "6",
                "orchestration.subscribeThread",
                json!({ "threadId": "task-gone", "requestCompletionMarker": true }),
            ),
            &state(),
            &viewer(),
        )
        .await;
    assert_eq!(output.frames[0]["_tag"], "Exit");
    assert_eq!(output.frames[0]["exit"]["cause"][0]["_tag"], "Die");
    assert!(!session.streams.contains("6"));
}

#[tokio::test]
async fn a_thread_subscription_without_a_thread_is_refused() {
    let mut session = session();
    let output = session
        .handle_json(
            &request("7", "orchestration.subscribeThread", json!({})),
            &state(),
            &viewer(),
        )
        .await;
    assert_eq!(output.frames[0]["_tag"], "Exit");
    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
}

/// The client decodes a method's failure against that method's declared
/// error type, so a tagged error is one of its ordinary outcomes and a
/// string defect is not. Answering "no favicon" with a defect leaves the
/// same blank icon behind a stack trace.
#[tokio::test]
async fn an_asset_this_environment_has_no_file_for_fails_in_the_contracts_own_type() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "8",
                "assets.createUrl",
                json!({ "resource": { "_tag": "project-favicon", "cwd": "/moatless" } }),
            ),
            &state(),
            &viewer(),
        )
        .await;
    let cause = &output.frames[0]["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail");
    assert_eq!(cause["error"]["_tag"], "AssetProjectFaviconNotFoundError");
    assert_eq!(
        cause["error"]["resource"],
        json!({ "_tag": "project-favicon", "cwd": "/moatless" }),
        "the error carries back the resource the request named"
    );
}

#[tokio::test]
async fn a_media_file_is_minted_rather_than_refused() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "9",
                "assets.createUrl",
                json!({ "resource": {
                    "_tag": "media-file", "threadId": "t-1",
                    "path": "/moatless/tasks/t-1/docs/shot.png"
                }}),
            ),
            &state(),
            &viewer(),
        )
        .await;
    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(
        output.frames[0]["exit"]["value"]["relativeUrl"],
        "/api/assets/t-1/docs/shot.png"
    );
}

/// The client reports focus on every change so a host can idle its own
/// background work. Moatless runs its agents in Sandboxes no browser tab is
/// attached to, so the report is accepted and dropped — refusing it would
/// put a defect in the console on every tab switch.
#[tokio::test]
async fn a_client_activity_report_is_accepted() {
    let mut session = session();
    let output = session
        .handle_json(
            &request(
                "9",
                "server.reportClientActivity",
                json!({ "state": "active" }),
            ),
            &state(),
            &viewer(),
        )
        .await;
    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(output.frames[0]["exit"]["value"], Value::Null);
}

/// The difference between a `Fail` and a `Die` is what the person sees: a
/// declared error renders its own message, a defect renders as "unexpected
/// server error" and nothing else. A method this server does not have has
/// not failed, and saying so is not a defect.
#[tokio::test]
async fn a_method_this_server_does_not_have_says_so_in_a_type_the_client_reads() {
    let mut session = session();
    let output = session
        .handle_json(
            &request("1", "orchestration.getFullThreadDiff", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    let cause = &output.frames[0]["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail", "a typed absence, not a Die defect");
    assert_eq!(cause["error"]["_tag"], "UnsupportedMethodError");
    // Carried separately from the message so a client fanning out several
    // calls can tell which one came back absent without parsing text.
    assert_eq!(cause["error"]["method"], "orchestration.getFullThreadDiff");
}

fn request_id(id: &str) -> RequestId {
    serde_json::from_value(json!(id)).expect("a string is a request id")
}

/// Found in production by `moatless_ui_rpc_requests_total{outcome="die"}`:
/// opening a thread the viewer cannot read answered `Die`, so the panel
/// could say only that the server was broken.
#[test]
fn a_thread_the_viewer_cannot_read_is_a_typed_refusal_not_a_defect() {
    let frame = sandbox_failure_exit(
        &request_id("1"),
        "sandbox.subscribeStatus",
        &SandboxRpcError::NotFound,
        &viewer(),
    );

    let cause = &frame["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail", "a refusal, not a Die defect");
    assert_eq!(cause["error"]["_tag"], "EnvironmentAuthorizationError");
    assert_eq!(cause["error"]["requiredScope"], "orchestration:read");
    assert_eq!(cause["error"]["message"], THREAD_UNAVAILABLE);
}

/// The narrowing only moves `NotFound`. A read that actually broke is still
/// a defect, because no client action answers it.
#[test]
fn a_sandbox_that_could_not_be_read_is_still_a_defect() {
    let frame = sandbox_failure_exit(
        &request_id("1"),
        "sandbox.status",
        &SandboxRpcError::Database(sqlx::Error::RowNotFound),
        &viewer(),
    );

    assert_eq!(frame["exit"]["cause"][0]["_tag"], "Die");
}

/// The stream's own ending, for a thread that stopped being readable while
/// it was open — the same refusal as the opening request's.
#[test]
fn a_subscription_ends_with_the_refusal_its_request_would_have_answered() {
    assert_eq!(
        sandbox_stream_refusal(&request_id("1"), "sandbox.subscribeStatus"),
        sandbox_failure_exit(
            &request_id("1"),
            "sandbox.subscribeStatus",
            &SandboxRpcError::NotFound,
            &viewer(),
        )
    );
}

/// The seam that found this now reads it as a refusal, so it stops counting
/// against the defect rate and starts counting against the refusal one.
#[test]
fn the_metrics_seam_reads_an_unreadable_thread_as_unauthorized() {
    let frames = vec![sandbox_failure_exit(
        &request_id("1"),
        "sandbox.subscribeStatus",
        &SandboxRpcError::NotFound,
        &viewer(),
    )];

    assert_eq!(
        rpc_metrics::classify(&frames),
        rpc_metrics::Outcome::Unauthorized
    );
}

/// Pairing advertises a scope list. Until this gate existed the list was
/// decoration — every method ran whatever it said.
#[tokio::test]
async fn a_method_needing_a_scope_this_deployment_withholds_is_refused() {
    let mut session = session();
    let output = session
        .handle_json(
            &request("1", "cloud.installRelayClient", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    let cause = &output.frames[0]["exit"]["cause"][0];
    assert_eq!(cause["_tag"], "Fail");
    assert_eq!(cause["error"]["_tag"], "EnvironmentAuthorizationError");
    assert_eq!(cause["error"]["requiredScope"], "relay:write");
}

/// Terminal metadata is a served stream. It starts with an explicit
/// empty snapshot so the client can distinguish "no terminals yet" from a
/// connection that never produced a value.
#[tokio::test]
async fn a_terminal_metadata_subscription_starts_with_a_snapshot() {
    let mut session = session();
    let output = session
        .handle_json(
            &request("7", "subscribeTerminalMetadata", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    assert!(session.streams.contains("7"));
    assert_eq!(output.frames[0]["_tag"], "Chunk");
    assert_eq!(output.frames[0]["values"][0]["type"], "snapshot");
    assert_eq!(output.frames[0]["values"][0]["terminals"], json!([]));
}

/// The client asking is already showing the conversation. Answering `null`
/// leaves it showing what the thread subscription gave it; failing would
/// take a rendered transcript off the screen over a missing title.
#[tokio::test]
async fn a_listing_row_for_a_conversation_this_viewer_cannot_reach_is_null_not_an_error() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request("1", "threads.getShell", json!({ "threadId": "task-1" })),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(output.frames[0]["exit"]["value"]["thread"], Value::Null);
}

/// Every client encodes against the contract, so a request without one is
/// malformed rather than a conversation that is missing — and the two must
/// not answer alike, or a client bug reads as an empty deployment.
#[tokio::test]
async fn a_listing_row_asked_for_without_a_thread_id_is_a_failure() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request("1", "threads.getShell", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
}

/// One character matches most of the corpus and says nothing, and it costs a
/// scan of every Message body to say so.
#[tokio::test]
async fn a_thread_search_too_short_to_mean_anything_never_reaches_the_database() {
    let mut connection = connection();
    // `state()` sets no `search_conversations` expectation, so the mock
    // panics if this reaches it — which is the assertion.
    let output = connection
        .exchange(
            &request("1", "orchestration.searchThreads", json!({ "query": "a" })),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(output.frames[0]["exit"]["value"]["matches"], json!([]));
}

#[tokio::test]
async fn interrupt_terminates_active_stream() {
    let mut session = session();
    // A subscription that stays open and says nothing — a thread with no
    // repository to report on.
    session
        .handle_json(
            &request("3", "subscribeVcsStatus", json!({})),
            &state(),
            &viewer(),
        )
        .await;
    assert!(session.streams.contains("3"));
    let output = session
        .handle_json(
            br#"{"_tag":"Interrupt","requestId":"3"}"#,
            &state(),
            &viewer(),
        )
        .await;
    assert!(!session.streams.contains("3"));
    assert_eq!(output.frames[0]["exit"]["cause"][0]["_tag"], "Interrupt");
}

#[tokio::test]
async fn a_subscription_that_ended_gives_its_slot_back() {
    let mut session = session();
    session
        .streams
        .start_produced(&request_id("3"), None, |_| async { Ok(()) });
    session.streams.wait_until_finished("3").await;

    session
        .handle_json(
            &request("4", "subscribeVcsStatus", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    assert!(!session.streams.contains("3"));
    assert!(session.streams.contains("4"));
}

#[tokio::test]
async fn a_request_id_can_be_reused_after_its_producer_finishes() {
    let mut session = session();
    session
        .streams
        .start_produced(&request_id("3"), None, |_| async { Ok(()) });
    session.streams.wait_until_finished("3").await;

    let output = session
        .handle_json(
            &request("3", "server.probe", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["_tag"], "Exit");
    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
}

#[tokio::test]
async fn a_connection_holding_live_subscriptions_is_refused_at_the_cap() {
    let mut session = session();
    for id in 0..MAX_ACTIVE_STREAMS {
        session
            .handle_json(
                &request(&id.to_string(), "subscribeVcsStatus", json!({})),
                &state(),
                &viewer(),
            )
            .await;
    }

    let output = session
        .handle_json(
            &request("9999", "subscribeVcsStatus", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["cause"][0]["_tag"], "Die");
    assert_eq!(session.streams.len(), MAX_ACTIVE_STREAMS);
}

#[tokio::test]
async fn a_json_array_is_one_effect_rpc_batch() {
    let mut session = session();
    let output = session.handle_json(
        br#"[{"_tag":"Ping"},{"_tag":"Request","id":"4","tag":"server.probe","payload":{},"headers":[]}]"#,
        &state(),
        &viewer(),
    ).await;
    assert_eq!(output.frames.len(), 2);
    assert_eq!(output.frames[0]["_tag"], "Pong");
    assert_eq!(output.frames[1]["_tag"], "Exit");
}

#[test]
fn origin_check_is_exact_and_requires_origin() {
    let allowed = vec![AllowedOrigin::parse("http://localhost:5733")];
    let mut headers = HeaderMap::new();
    assert!(!origin_allowed(&headers, &allowed));
    headers.insert(header::ORIGIN, "http://localhost:5733".parse().unwrap());
    assert!(origin_allowed(&headers, &allowed));
    headers.insert(header::ORIGIN, "http://attacker.example".parse().unwrap());
    assert!(!origin_allowed(&headers, &allowed));
}

/// Both file methods answer in the shape their own contract declares, and
/// not as a defect. A `Die` reaches the file panel as "Workspace query
/// failed." with the reason stripped, so a refusal the person can act on —
/// no sandbox — has to arrive as a `Fail` carrying the sentence to show.
#[tokio::test]
async fn a_refused_file_request_fails_with_the_method_s_own_error() {
    for (tag, payload, error_tag) in [
        (
            "projects.listEntries",
            json!({ "cwd": "/moatless/workspaces/ws-a" }),
            "ProjectListEntriesError",
        ),
        (
            "projects.readFile",
            json!({ "cwd": "/moatless/workspaces/ws-a", "relativePath": "README.md" }),
            "ProjectReadFileError",
        ),
    ] {
        let mut connection = connection();
        let output = connection
            .exchange(&request("7", tag, payload), &state(), &viewer())
            .await;
        let exit = &output.frames[0]["exit"];
        assert_eq!(exit["_tag"], "Failure", "{tag}");
        assert_eq!(exit["cause"][0]["_tag"], "Fail", "{tag}");
        assert_eq!(exit["cause"][0]["error"]["_tag"], error_tag);
        assert_eq!(
            exit["cause"][0]["error"]["message"],
            "This conversation has no sandbox, so it has no workspace yet.",
            "a project root is the cwd a thread with no worktree sends"
        );
    }
}

/// A `cwd` naming a conversation the viewer's listing does not contain is
/// answered the same way as one naming nothing at all. The stub listing is
/// empty, so this is every conversation.
#[tokio::test]
async fn a_cwd_outside_the_viewer_s_listing_is_indistinguishable_from_an_unknown_one() {
    for cwd in [
        "/moatless/tasks/task-a",
        "/somewhere/else",
        "/moatless/tasks/",
    ] {
        let mut connection = connection();
        let output = connection
            .exchange(
                &request("8", "projects.listEntries", json!({ "cwd": cwd })),
                &state(),
                &viewer(),
            )
            .await;
        let exit = &output.frames[0]["exit"];
        assert_eq!(exit["cause"][0]["_tag"], "Fail", "{cwd}");
        assert_eq!(
            exit["cause"][0]["error"]["message"], "This conversation's workspace is not available.",
            "{cwd} must not report whether the conversation exists"
        );
    }
}

#[test]
fn enabled_configuration_requires_an_origin() {
    let config = UiRpcConfig {
        environment_id: "moatless".into(),
        allowed_origins: vec![],
        otlp_traces_enabled: false,
        attachment_uploads: true,
        usage_limit_sources: false,
    };
    assert!(config.validate().is_err());
}

/// A client refuses a server whose version differs from its own, so the
/// descriptor answers with the contract constant.
#[test]
fn the_descriptor_reports_the_contract_version_the_code_implements() {
    let config = UiRpcConfig {
        environment_id: "moatless".into(),
        allowed_origins: vec![AllowedOrigin::parse("https://example.com")],
        otlp_traces_enabled: false,
        attachment_uploads: true,
        usage_limit_sources: false,
    };

    assert_eq!(
        descriptor(&config)["serverVersion"],
        json!(T3_CLIENT_CONTRACT_VERSION)
    );
    assert_ne!(
        T3_CLIENT_CONTRACT_VERSION,
        env!("CARGO_PKG_VERSION"),
        "the contract version tracks the T3 client, not this crate"
    );
}

#[test]
fn a_ticket_holder_needs_no_origin_and_everyone_else_does() {
    assert!(!origin_checked(Some(Extension(
        UiRpcAuthMethod::WebSocketTicket
    ))));
    assert!(origin_checked(Some(Extension(
        UiRpcAuthMethod::SessionCookie
    ))));
    assert!(origin_checked(Some(Extension(UiRpcAuthMethod::ApiKey))));
    // An upgrade the middleware never saw is treated as the cookie case.
    assert!(origin_checked(None));
}

/// The route a browser frame fetches a workspace file from.
mod workspace_assets {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn text(response: Response) -> String {
        let body = axum::body::to_bytes(response.into_body(), MAX_FRAME_BYTES)
            .await
            .unwrap();
        String::from_utf8(body.to_vec()).unwrap()
    }

    /// A minted URL is an address and not a capability, so the Task behind
    /// it is re-read through the same follow-and-access filter on every
    /// fetch. The stub listing holds no conversations, so this viewer
    /// follows none — and is told what someone asking for a Task that does
    /// not exist is told.
    #[tokio::test]
    async fn a_conversation_this_viewer_does_not_follow_serves_no_file() {
        let mut request = Request::builder()
            .uri("/api/assets/t-1/docs/report.html")
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(viewer());

        let response = router(state()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        // The route is mounted and refused it, rather than never matching.
        assert_eq!(
            text(response).await,
            crate::workspace::WorkspaceAccessError::Unaddressable.message()
        );
    }
}

/// The JSON read routes negotiate compression; the socket does not.
mod http_compression {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn shell(accept_encoding: Option<&str>) -> Response {
        let mut builder = Request::builder().uri("/api/orchestration/shell");
        if let Some(encoding) = accept_encoding {
            builder = builder.header("accept-encoding", encoding);
        }
        let mut request = builder.body(Body::empty()).unwrap();
        request.extensions_mut().insert(viewer());
        router(state()).oneshot(request).await.unwrap()
    }

    /// A client that offers gzip gets it: the layer is wired to this route,
    /// which is the ~5x saving on the thread bootstrap the client pulls over
    /// HTTP for exactly this reason.
    #[tokio::test]
    async fn a_client_that_offers_gzip_gets_a_gzipped_body() {
        let response = shell(Some("gzip")).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(header::CONTENT_ENCODING)
                .and_then(|value| value.to_str().ok()),
            Some("gzip")
        );
    }

    /// A client that offers nothing still gets a readable body: compression
    /// is negotiated, never forced on a client that cannot decode it.
    #[tokio::test]
    async fn a_client_that_offers_no_encoding_gets_an_uncompressed_body() {
        let response = shell(None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get(header::CONTENT_ENCODING).is_none());
    }
}

/// The upload route, driven the way the composer drives it: mint a URL over
/// the socket, then `POST` the raw file to it.
mod attachment_uploads {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    /// Mint a URL for an attachment of `size` bytes and return the path to
    /// `POST` it to.
    async fn minted_url(state: &UiRpcState, size: usize) -> String {
        let frame = request(
            "1",
            "attachments.createUploadUrl",
            json!({
                "type": "file",
                "name": "report.pdf",
                "mimeType": "application/pdf",
                "sizeBytes": size
            }),
        );
        let mut session = session();
        let output = session.handle_json(&frame, state, &viewer()).await;
        output.frames[0]["exit"]["value"]["relativeUrl"]
            .as_str()
            .expect("a minted url")
            .to_string()
    }

    async fn upload(state: UiRpcState, url: &str, body: Vec<u8>) -> StatusCode {
        let request = Request::builder()
            .method("POST")
            .uri(url)
            .header("content-type", "application/pdf")
            .body(Body::from(body))
            .expect("a request");
        router(state)
            .oneshot(request)
            .await
            .expect("a response")
            .status()
    }

    /// The whole point of the route: bytes in, and the name and type the
    /// mint declared — not the ones the request carried — stored beside
    /// them under the id the client is already holding.
    #[tokio::test]
    async fn a_body_of_the_declared_size_is_stored_under_the_minted_id() {
        let attachments = Arc::new(RecordingAttachments::default());
        let state = state_with_attachments(attachments.clone());
        let url = minted_url(&state, 8).await;
        let id = url.rsplit('/').next().unwrap_or_default().to_string();

        assert_eq!(
            upload(state, &url, vec![7u8; 8]).await,
            StatusCode::NO_CONTENT
        );
        #[allow(clippy::unwrap_used)]
        let stored = attachments.stored.lock().unwrap().clone();
        assert_eq!(stored.len(), 1, "{stored:?}");
        assert_eq!(stored[0].1, "report.pdf");
        assert_eq!(stored[0].2, "application/pdf");
        assert_eq!(stored[0].3, 8);
        // The id is in the token, not in the path — the path *is* the
        // token — so this asserts the two are the same string.
        assert!(!id.is_empty());
        assert!(url.ends_with(&id));
    }

    /// The boundary, from both sides. A body one byte over its declaration
    /// is refused and stores nothing; a body of exactly the declared size
    /// is stored.
    #[tokio::test]
    async fn a_body_is_refused_at_the_first_byte_past_its_declaration() {
        let attachments = Arc::new(RecordingAttachments::default());
        let state = state_with_attachments(attachments.clone());
        let url = minted_url(&state, 8).await;

        assert_eq!(
            upload(state.clone(), &url, vec![7u8; 9]).await,
            StatusCode::BAD_REQUEST
        );
        #[allow(clippy::unwrap_used)]
        let stored = attachments.stored.lock().unwrap().len();
        assert_eq!(stored, 0, "an oversize body must not be stored");

        assert_eq!(
            upload(state, &url, vec![7u8; 8]).await,
            StatusCode::NO_CONTENT
        );
    }

    /// A body that ends short of its declaration is a truncated upload, and
    /// storing it would reach the agent as a corrupt file.
    #[tokio::test]
    async fn a_body_that_ends_short_of_its_declaration_is_refused() {
        let attachments = Arc::new(RecordingAttachments::default());
        let state = state_with_attachments(attachments.clone());
        let url = minted_url(&state, 8).await;

        assert_eq!(
            upload(state, &url, vec![7u8; 7]).await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(attachments.stored.lock().expect("no poison").len(), 0);
    }

    /// The token in the path is the credential. A forged one, an edited one
    /// and one from another deployment all write nothing — this route is
    /// exempt from the auth middleware precisely because it authenticates
    /// itself.
    #[tokio::test]
    async fn a_url_this_server_did_not_mint_stores_nothing() {
        let attachments = Arc::new(RecordingAttachments::default());
        let state = state_with_attachments(attachments.clone());
        let minted = minted_url(&state, 8).await;

        for url in [
            "/api/attachments/not-a-token".to_string(),
            format!("{minted}x"),
        ] {
            assert_eq!(
                upload(state.clone(), &url, vec![7u8; 8]).await,
                StatusCode::UNAUTHORIZED,
                "{url}"
            );
        }
        assert_eq!(attachments.stored.lock().expect("no poison").len(), 0);
    }
}

/// The pairing exchange and the ticket route, over HTTP, as a client on
/// another device drives them.
mod pairing {
    use super::*;
    use crate::auth::verify_ws_ticket_jwt;
    use axum::body::Body;
    use axum::http::Request;
    use chrono::Utc;
    use tower::ServiceExt;

    const TEST_SECRET: &str = "test-secret";

    fn state_pairing(pairing: DevicePairing) -> UiRpcState {
        UiRpcState {
            pairing: Arc::new(pairing),
            ..state()
        }
    }

    fn exchange(body: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    /// What the T3 mobile client sends, byte for byte.
    ///
    /// Captured off `bootstrapRemoteBearerSession` driven with the mobile
    /// app's own scopes and metadata, rather than written from the schema:
    /// the encoding is the part a hand-written fixture gets wrong, and it
    /// is `+` for a space and `%3A` for the colons in every URN.
    fn mobile_exchange(code: &str) -> Request<Body> {
        exchange(&format!(
            "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Atoken-exchange\
             &subject_token={code}\
             &subject_token_type=urn%3At3%3Aparams%3Aoauth%3Atoken-type%3Aenvironment-bootstrap\
             &requested_token_type=urn%3Aietf%3Aparams%3Aoauth%3Atoken-type%3Aaccess_token\
             &scope=orchestration%3Aread+orchestration%3Aoperate+terminal%3Aoperate+review%3Awrite+relay%3Aread\
             &client_label=T3+Code+Mobile&client_device_type=mobile&client_os=iOS"
        ))
    }

    async fn json(response: Response) -> Value {
        let body = axum::body::to_bytes(response.into_body(), MAX_FRAME_BYTES)
            .await
            .unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn a_typed_code_becomes_the_token_the_device_keeps() {
        let pairing = test_pairing(Some(pairing_code("ABCDEFGHJKLM")));

        let response = router(state_pairing(pairing))
            .oneshot(mobile_exchange("ABCDEFGHJKLM"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = json(response).await;
        assert_eq!(body["token_type"], "Bearer");
        assert_eq!(
            body["issued_token_type"],
            "urn:ietf:params:oauth:token-type:access_token"
        );
        assert!(body["access_token"].as_str().unwrap().starts_with("mvk_"));
        // The overlap between what it asked for and what we have.
        assert_eq!(
            body["scope"],
            "orchestration:read orchestration:operate terminal:operate review:write relay:read"
        );
        assert!(body["expires_in"].as_i64().unwrap() > 0);
    }

    #[tokio::test]
    async fn a_code_that_cannot_be_redeemed_is_refused_the_way_t3_reads() {
        // Never minted, already spent, expired — one answer for all three.
        let pairing = test_pairing(None);

        let response = router(state_pairing(pairing))
            .oneshot(mobile_exchange("ABCDEFGHJKLM"))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = json(response).await;
        assert_eq!(body["_tag"], "EnvironmentAuthInvalidError");
        assert_eq!(body["code"], "auth_invalid");
        assert_eq!(body["reason"], "invalid_credential");
        assert!(!body["traceId"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_grant_this_environment_does_not_offer_never_reaches_the_code() {
        let response = router(state_pairing(test_pairing(Some(pairing_code(
            "ABCDEFGHJKLM",
        )))))
        .oneshot(exchange(
            "grant_type=authorization_code&subject_token=ABCDEFGHJKLM\
                 &subject_token_type=urn%3At3%3Aparams%3Aoauth%3Atoken-type%3Aenvironment-bootstrap\
                 &requested_token_type=urn%3Aietf%3Aparams%3Aoauth%3Atoken-type%3Aaccess_token",
        ))
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let body = json(response).await;
        assert_eq!(body["_tag"], "EnvironmentRequestInvalidError");
        assert_eq!(body["reason"], "invalid_command");
    }

    #[tokio::test]
    async fn a_paired_client_trades_its_token_for_a_ticket() {
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/auth/websocket-ticket")
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(viewer());

        let response = router(state()).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = json(response).await;
        let claims = verify_ws_ticket_jwt(body["ticket"].as_str().unwrap(), TEST_SECRET).unwrap();
        assert_eq!(claims.sub, viewer().user_id);
        // T3 decodes this as a `DateTimeUtc`, so it has to be an instant.
        assert!(
            body["expiresAt"]
                .as_str()
                .unwrap()
                .parse::<chrono::DateTime<Utc>>()
                .is_ok()
        );
    }

    #[tokio::test]
    async fn a_deployment_with_no_signing_secret_says_so() {
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/auth/websocket-ticket")
            .body(Body::empty())
            .unwrap();
        request.extensions_mut().insert(viewer());

        let unsigned = UiRpcState {
            session_jwt_secret: None,
            ..state()
        };
        let response = router(unsigned).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(
            json(response).await["reason"],
            "websocket_ticket_issuance_failed"
        );
    }
}

mod trace_context {
    use super::*;
    use opentelemetry::trace::{TraceContextExt, TracerProvider};
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
    use tracing_subscriber::layer::SubscriberExt;

    const VALID_TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";
    const VALID_SPAN_ID: &str = "b7ad6b7169203331";

    #[tokio::test]
    async fn otlp_traces_enabled_reflects_whether_export_is_active() {
        let off = state();
        assert_eq!(
            server_config(&off, &viewer(), &[]).await["observability"]["otlpTracesEnabled"],
            false
        );

        let on = UiRpcState {
            rpc: Arc::new(UiRpcConfig {
                otlp_traces_enabled: true,
                ..(*off.rpc).clone()
            }),
            ..off
        };
        assert_eq!(
            server_config(&on, &viewer(), &[]).await["observability"]["otlpTracesEnabled"],
            true
        );
    }

    #[test]
    fn well_formed_client_context_parses_into_a_remote_span_context() {
        let cx = remote_span_context(Some(VALID_TRACE_ID), Some(VALID_SPAN_ID), Some(true))
            .expect("well-formed ids parse");
        let span_context = cx.span().span_context().clone();
        assert_eq!(span_context.trace_id().to_string(), VALID_TRACE_ID);
        assert_eq!(span_context.span_id().to_string(), VALID_SPAN_ID);
        assert!(span_context.is_sampled());
        assert!(span_context.is_remote());
    }

    #[test]
    fn unsampled_client_context_carries_no_sampled_flag() {
        let cx = remote_span_context(Some(VALID_TRACE_ID), Some(VALID_SPAN_ID), Some(false))
            .expect("well-formed ids parse");
        assert!(!cx.span().span_context().is_sampled());
    }

    #[test]
    fn missing_ids_yield_no_remote_context() {
        assert!(remote_span_context(None, Some(VALID_SPAN_ID), Some(true)).is_none());
        assert!(remote_span_context(Some(VALID_TRACE_ID), None, Some(true)).is_none());
    }

    #[test]
    fn malformed_ids_yield_no_remote_context() {
        // Wrong length.
        assert!(remote_span_context(Some("deadbeef"), Some(VALID_SPAN_ID), Some(true)).is_none());
        // Right length, not hex.
        assert!(
            remote_span_context(
                Some("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz"),
                Some(VALID_SPAN_ID),
                Some(true)
            )
            .is_none()
        );
        // The all-zero trace id is explicitly invalid per the W3C spec.
        assert!(
            remote_span_context(
                Some("00000000000000000000000000000000"),
                Some(VALID_SPAN_ID),
                Some(true)
            )
            .is_none()
        );
    }

    #[test]
    fn a_well_formed_client_trace_id_becomes_the_request_spans_trace_id() {
        let (provider, exporter) = {
            let exporter = InMemorySpanExporter::default();
            let provider = SdkTracerProvider::builder()
                .with_simple_exporter(exporter.clone())
                .build();
            (provider, exporter)
        };
        let otel_layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
        let subscriber = tracing_subscriber::registry().with(otel_layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        {
            let span = request_root_span(
                "shell.list",
                Some(VALID_TRACE_ID),
                Some(VALID_SPAN_ID),
                Some(true),
            );
            let _entered = span.enter();
        }
        provider.force_flush().expect("flush the simple exporter");

        let spans = exporter.get_finished_spans().expect("exporter is readable");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].span_context.trace_id().to_string(), VALID_TRACE_ID);
    }

    #[test]
    fn a_malformed_client_trace_id_mints_a_fresh_root_and_records_the_raw_value() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let otel_layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
        let subscriber = tracing_subscriber::registry().with(otel_layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        {
            let span = request_root_span("shell.list", Some("not-a-trace-id"), None, None);
            let _entered = span.enter();
        }
        provider.force_flush().expect("flush the simple exporter");

        let spans = exporter.get_finished_spans().expect("exporter is readable");
        assert_eq!(spans.len(), 1);
        // A fresh root never shares the malformed client value verbatim as
        // its trace id, but the raw value is preserved as an attribute.
        assert_ne!(
            spans[0].span_context.trace_id().to_string(),
            "not-a-trace-id"
        );
        let recorded = spans[0]
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "client_trace_id");
        assert_eq!(
            recorded.map(|kv| kv.value.as_str().to_string()),
            Some("not-a-trace-id".to_string())
        );
    }

    /// A `Die` reaches Tempo as an error span, which is what makes "show me
    /// the failing requests" a search rather than a read of every span.
    #[test]
    fn a_failed_request_marks_its_span_as_an_error() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let otel_layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
        let subscriber = tracing_subscriber::registry().with(otel_layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        {
            let span = request_root_span("shell.list", None, None, None);
            let _entered = span.enter();
            record_request(
                &span,
                "shell.list",
                &[json!({
                    "_tag": "Exit",
                    "requestId": "1",
                    "exit": { "_tag": "Failure", "cause": [{ "_tag": "Die", "defect": "boom" }] }
                })],
                Duration::from_millis(3),
                &viewer(),
            );
        }
        provider.force_flush().expect("flush the simple exporter");

        let spans = exporter.get_finished_spans().expect("exporter is readable");
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].status, opentelemetry::trace::Status::error(""));
        let outcome = spans[0]
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "outcome");
        assert_eq!(
            outcome.map(|kv| kv.value.as_str().to_string()),
            Some("die".to_string())
        );
    }

    /// The counterpart: a success must not be searchable as an error, and
    /// it still carries the outcome so a Tempo search can name it.
    #[test]
    fn a_successful_request_leaves_its_span_unset() {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_simple_exporter(exporter.clone())
            .build();
        let otel_layer = tracing_opentelemetry::layer().with_tracer(provider.tracer("test"));
        let subscriber = tracing_subscriber::registry().with(otel_layer);
        let _guard = tracing::subscriber::set_default(subscriber);

        {
            let span = request_root_span("server.probe", None, None, None);
            let _entered = span.enter();
            record_request(
                &span,
                "server.probe",
                &[json!({
                    "_tag": "Exit",
                    "requestId": "1",
                    "exit": { "_tag": "Success", "value": {} }
                })],
                Duration::from_millis(1),
                &viewer(),
            );
        }
        provider.force_flush().expect("flush the simple exporter");

        let spans = exporter.get_finished_spans().expect("exporter is readable");
        assert_eq!(spans.len(), 1);
        assert_ne!(spans[0].status, opentelemetry::trace::Status::error(""));
        let outcome = spans[0]
            .attributes
            .iter()
            .find(|kv| kv.key.as_str() == "outcome");
        assert_eq!(
            outcome.map(|kv| kv.value.as_str().to_string()),
            Some("success".to_string())
        );
    }
}
