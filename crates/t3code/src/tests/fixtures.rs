//! The server state, connection and viewer every case here runs against, and
//! the stub ports that stand in for everything outside this crate.

use super::*;

/// The stub's database. Every read in these tests reports it, so nothing here
/// exercises the identity guard — that needs two real databases, and is in
/// `backend/tests/integration_surface/ui_rpc_shell_test.rs`.
pub(super) fn stub_identity() -> UiRpcEnvironmentIdentity {
    UiRpcEnvironmentIdentity::new("7667539899605483559:16384")
}

/// A deployment whose Sandboxes are all unreachable. Every request is refused
/// before reaching one, so these seams are held, never called — a live
/// Sandbox's answer belongs in an integration test.
struct UnreachableSandboxes;

fn unreachable(task_id: &str) -> SandboxError {
    SandboxError::NotReady(task_id.to_string())
}

pub(super) fn unreachable_sandboxes() -> Arc<dyn sandbox_proto::client::SandboxConnector> {
    Arc::new(UnreachableSandboxes)
}

pub(super) fn unreachable_ports() -> Arc<dyn SandboxPortPublisher> {
    Arc::new(UnreachableSandboxes)
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

pub(super) fn test_pairing(code: Option<PairingCode>) -> DevicePairing {
    device_pairing_over(code, Arc::new(TestApiKeyIssuer))
}

pub(super) fn pairing_code(code: &str) -> PairingCode {
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

/// The full catalog with everything installed. The provider list's own shape is
/// asserted in `providers.rs`; here it only has to be non-empty so the composer
/// is not disabled.
struct StubAgents;

#[async_trait]
impl ports::AgentCatalogSource for StubAgents {
    async fn list(
        &self,
        _viewer: &ScopeContext,
    ) -> Vec<moatless_api_schemas::agent::AgentDescriptor> {
        crate::agent::catalog::catalog()
            .map(|entry| moatless_api_schemas::agent::AgentDescriptor {
                id: entry.id().to_string(),
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
/// git-host branch listing is never reached here; it is asserted in `review.rs`
/// against a mock workspace DAO and branch source.
struct UnreachableBranches;

#[async_trait]
impl ports::RepositoryBranchSource for UnreachableBranches {
    async fn list_branches(
        &self,
        _viewer: &ScopeContext,
        _repository_id: &str,
        _workspace_id: Option<&str>,
    ) -> Result<
        moatless_workspace_types::schemas::repository::BranchListResponse,
        ports::BranchListError,
    > {
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

    async fn parent(
        &self,
        _child_task_id: &str,
    ) -> Result<Option<ports::ParentTask>, moatless_types::store::StoreError> {
        Ok(None)
    }
}

/// What reached the attachment store, so the mint, the upload route and
/// `attachments.delete` can be asserted without a database or an S3 bucket
/// behind them.
#[derive(Default)]
pub(super) struct RecordingAttachments {
    /// Each stored attachment as `(id, filename, mime type, byte count)`.
    pub(super) stored: std::sync::Mutex<Vec<(String, String, String, usize)>>,
    pub(super) deleted: std::sync::Mutex<Vec<String>>,
    pub(super) refuse: Option<ports::AttachmentStoreError>,
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

/// Nothing in this file dispatches a command that reaches a write; the command
/// table itself is asserted in `commands.rs` against a recording sink.
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
    async fn revert(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
        _keep_through: i32,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file rewinds a thread")
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
    async fn set_visibility(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
        _visibility: moatless_types::task::Visibility,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file changes visibility")
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
    async fn follow(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file follows a thread")
    }
    async fn unfollow(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file unfollows a thread")
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
    async fn steer_queued(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
        _message_uuid: &str,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file steers a queued turn")
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
    async fn link_pull_request(
        &self,
        _viewer: &ScopeContext,
        _reference: ports::ThreadPullRequestRef,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file links a pull request")
    }
    async fn unlink_pull_request(
        &self,
        _viewer: &ScopeContext,
        _reference: ports::ThreadPullRequestRef,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file unlinks a pull request")
    }
    async fn change_queue(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
        _change: ports::QueueChange,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file changes a queue")
    }
    async fn visit(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
        _visited_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file visits a thread")
    }
    async fn mark_unread(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file marks a thread unread")
    }
    async fn delete(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file deletes a thread")
    }
    async fn pin(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
        _order_key: Option<String>,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file changes per-viewer thread state")
    }
    async fn unpin(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file changes per-viewer thread state")
    }
    async fn set_pin_order_key(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
        _order_key: String,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file changes per-viewer thread state")
    }
    async fn set_active_order_key(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
        _order_key: String,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file changes per-viewer thread state")
    }
    async fn watch_pull_request(
        &self,
        _viewer: &ScopeContext,
        _reference: ports::ThreadPullRequestRef,
        _watching: bool,
    ) -> Result<(), ports::CommandRefusal> {
        unreachable!("no test in this file watches a pull request")
    }
}

/// These tests are about the envelope encoding, so the listing behind it is a
/// stub. What a viewer actually sees is asserted against a real database in
/// `backend/tests/integration_surface/ui_rpc_shell_test.rs`.
pub(super) fn state() -> UiRpcState {
    state_with_attachments(Arc::new(RecordingAttachments::default()))
}

pub(super) fn state_with_attachments(attachments: Arc<dyn ports::AttachmentStore>) -> UiRpcState {
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
    conversations
        .expect_can_read_conversation()
        .returning(|_, _| Ok(false));
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
    // Lazy, so nothing connects. No route these tests drive reaches the two
    // reads that take the pool; `task_creator` and `attachment_is_on_message`
    // are asserted against PostgreSQL in `dao_db_tests.rs`.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgres://stub@stub/stub")
        .expect("a lazy pool parses its URL and connects to nothing");
    UiRpcState {
        rpc: Arc::new(UiRpcConfig {
            environment_id: environment_id(&stub_identity()),
            allowed_origins: vec![AllowedOrigin::parse("http://localhost:5733")],
            otlp_traces_enabled: false,
            attachment_uploads: true,
            usage_limit_sources: false,
            orchestration_protocol_version: 1,
        }),
        environment: Arc::new(stub_identity()),
        session_cookie_name: Arc::from("moatless_session"),
        frontend_url: Arc::from("http://localhost:3000"),
        session_jwt_secret: Some(Arc::from("test-secret")),
        pairing: Arc::new(test_pairing(None)),
        reads: T3Reads::new(
            pool,
            conversations.clone(),
            Arc::new(workspaces),
            Arc::new(MockT3TaskDao::new()),
        ),
        workspace_script_commands: Arc::new(ports::MockWorkspaceScriptCommands::new()),
        sandboxes: Arc::new(UnreachableSandboxes),
        snapshots: None,
        scripts: ScriptsState {
            workspaces: Arc::new(MockT3WorkspaceDao::new()),
            ports: Arc::new(UnreachableSandboxes),
            preview: PreviewState {
                tabs: Arc::new(preview_tabs::PreviewTabs::new(preview_tabs::PreviewTabTtl(
                    std::time::Duration::from_hours(1),
                ))),
                tasks: Arc::new(MockT3TaskDao::new()),
                relay: preview_relay::PreviewRelay::disabled(),
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
            tabs: Arc::new(preview_tabs::PreviewTabs::new(preview_tabs::PreviewTabTtl(
                std::time::Duration::from_hours(1),
            ))),
            tasks: Arc::new(MockT3TaskDao::new()),
            relay: preview_relay::PreviewRelay::disabled(),
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
        client_traces: observability::ClientTraceForwarder::new(
            None,
            moatless_core::config::HttpConfig::default().client_traces,
        ),
        usage_limit_sources: crate::usage_limit_sources::UsageLimitSources::disabled(),
    }
}

/// A session whose pushed frames go nowhere; these tests drive requests only. A
/// detached request answers on this channel, not its caller — use
/// [`connection`] for tests on a `detach`-routed tag.
pub(super) fn session() -> ProtocolSession {
    ProtocolSession::new(mpsc::channel(OUTBOUND_FRAMES).0)
}

/// A session holding the frame channel `serve_websocket` writes to the socket.
pub(super) struct Connection {
    pub(super) session: ProtocolSession,
    pushed: mpsc::Receiver<Value>,
}

pub(super) fn connection() -> Connection {
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
    pub(super) async fn exchange(
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

    pub(super) fn next_pushed(&mut self) -> Option<Value> {
        self.pushed.try_recv().ok()
    }

    /// Wait for the next frame a subscription's producer task pushed.
    ///
    /// A produced stream's first frame is written by a `tokio::spawn`, which no
    /// `wait_until_idle` covers, so [`Connection::next_pushed`] races it.
    pub(super) async fn next_streamed(&mut self) -> Value {
        tokio::time::timeout(std::time::Duration::from_secs(5), self.pushed.recv())
            .await
            .expect("the producer pushed a frame")
            .expect("the frame channel is open")
    }
}

/// A Task projection that answers for `readable` and for nothing else, which is
/// what a scoped read does for a viewer without access.
pub(super) fn tasks_readable(readable: &'static str) -> Arc<dyn crate::task_dao::T3TaskDao> {
    let mut tasks = MockT3TaskDao::new();
    tasks.expect_find_by_id().returning(move |task_id, _| {
        // Inert but for having answered: the gate reads whether a row came
        // back, never a column on it.
        Ok((task_id == readable).then_some(TaskView {
            status: moatless_types::task::TaskStatus::Open,
            ttl_minutes: 15,
        }))
    });
    Arc::new(tasks)
}

/// One terminal's entry as the hub stores and broadcasts it.
pub(super) fn terminal_summary_for(thread_id: &str, terminal_id: &str) -> Value {
    json!({
        "threadId": thread_id,
        "terminalId": terminal_id,
        "cwd": "/moatless/tasks/task-1",
        "label": "Terminal 1",
        "status": "running",
    })
}

pub(super) fn viewer() -> UiRpcAuthIdentity {
    UiRpcAuthIdentity {
        user_id: Uuid::nil(),
        login: "viewer".into(),
        role: UserRole::User,
        provider: "test".into(),
        group_id: None,
    }
}

pub(super) fn request(id: &str, tag: &str, payload: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "_tag": "Request",
        "id": id,
        "tag": tag,
        "payload": payload,
        "headers": []
    }))
    .unwrap()
}

pub(super) fn request_id(id: &str) -> RequestId {
    serde_json::from_value(json!(id)).expect("a string is a request id")
}
