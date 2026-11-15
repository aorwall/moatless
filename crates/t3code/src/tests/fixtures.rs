//! The server state, connection and viewer every case here runs against, and
//! the stub ports that stand in for everything outside this crate.

use super::*;

/// The stub's database. Every read in these tests reports it, so nothing here
/// exercises the identity guard — that needs two real databases, and is in
/// `backend/tests/integration_surface/ui_rpc_shell_test.rs`.
pub(super) fn stub_identity() -> EnvironmentIdentity {
    EnvironmentIdentity::new("7667539899605483559:16384")
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
impl PairedDeviceKeys for TestApiKeyIssuer {
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

    async fn list_paired_keys(
        &self,
        _user_id: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<PairedDeviceKey>, PairingError> {
        Ok(vec![PairedDeviceKey {
            id: "key-1".into(),
            name: "T3 Code Mobile (iOS)".into(),
            created_at: now,
            expires_at: now + chrono::Duration::days(90),
            last_used_at: None,
        }])
    }

    async fn delete_paired_key(&self, _user_id: &str, key_id: &str) -> Result<bool, PairingError> {
        Ok(key_id == "key-1")
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

/// No Task in these tests is bound to a pull request, so the host behind
/// `pullRequests.detail` and the guided review is never reached.
struct UnreachablePullRequestReviews;

#[async_trait]
impl ports::PullRequestReviewSource for UnreachablePullRequestReviews {
    async fn project_repository(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: &str,
    ) -> Result<Option<String>, ports::PullRequestError> {
        unreachable!("no test in this file lists pull requests")
    }
    async fn task_repositories(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: &str,
    ) -> Result<Vec<String>, ports::PullRequestError> {
        unreachable!("no test in this file reads a pull request")
    }
    async fn list(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: &str,
        _repository: &str,
        _query: &ports::PullRequestListQuery,
        _limit: u32,
        _cursor: Option<&str>,
    ) -> Result<ports::PullRequestListPage, ports::PullRequestError> {
        unreachable!("no test in this file lists pull requests")
    }
    async fn overview(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: Option<&str>,
        _repository: &str,
        _number: u64,
    ) -> Result<ports::PullRequestOverview, ports::PullRequestError> {
        unreachable!("no test in this file reads a bound pull request")
    }
    async fn patch_page(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: Option<&str>,
        _repository: &str,
        _number: u64,
        _page: u32,
    ) -> Result<ports::PullRequestPatchPage, ports::PullRequestError> {
        unreachable!("no test in this file reads a bound pull request")
    }
    async fn commit_patch_page(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: Option<&str>,
        _repository: &str,
        _sha: &str,
        _page: u32,
    ) -> Result<ports::PullRequestPatchPage, ports::PullRequestError> {
        unreachable!("no test in this file reads a bound pull request")
    }
    async fn activity(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: Option<&str>,
        _repository: &str,
        _number: u64,
    ) -> Result<
        moatless_types::pull_request_conversation::PullRequestActivity,
        ports::PullRequestError,
    > {
        unreachable!("no test in this file reads a bound pull request")
    }
    async fn thread_comments(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: Option<&str>,
        _repository: &str,
        _number: u64,
        _thread_id: &str,
        _cursor: Option<&str>,
    ) -> Result<
        moatless_types::pull_request_conversation::ReviewThreadCommentPage,
        ports::PullRequestError,
    > {
        unreachable!("no test in this file reads a bound pull request")
    }
    async fn status(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: Option<&str>,
        _repository: &str,
        _number: u64,
    ) -> Result<moatless_types::pull_request_conversation::PullRequestStatus, ports::PullRequestError>
    {
        unreachable!("no test in this file reads a bound pull request")
    }
    async fn write(
        &self,
        _viewer: &ScopeContext,
        _repository: &str,
        _number: u64,
        _write: &moatless_types::pull_request_conversation::PullRequestWrite,
    ) -> Result<(), ports::PullRequestError> {
        unreachable!("no test in this file writes to a pull request")
    }
    async fn label_candidates(
        &self,
        _viewer: &ScopeContext,
        _repository: &str,
        _number: u64,
    ) -> Result<
        moatless_types::pull_request_conversation::Candidates<
            moatless_types::pull_request_conversation::PullRequestLabelCandidate,
        >,
        ports::PullRequestError,
    > {
        unreachable!("no test in this file writes to a pull request")
    }
    async fn reviewer_candidates(
        &self,
        _viewer: &ScopeContext,
        _repository: &str,
        _number: u64,
        _query: Option<&str>,
    ) -> Result<
        moatless_types::pull_request_conversation::Candidates<
            moatless_types::pull_request_conversation::PullRequestReviewerCandidate,
        >,
        ports::PullRequestError,
    > {
        unreachable!("no test in this file writes to a pull request")
    }
    async fn file_stats(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: Option<&str>,
        _repository: &str,
        _number: u64,
    ) -> Result<
        Option<Vec<moatless_types::pull_request_conversation::PullRequestFileStat>>,
        ports::PullRequestError,
    > {
        unreachable!("no test in this file writes to a pull request")
    }
    async fn file_contents(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: Option<&str>,
        _repository: &str,
        _number: u64,
        _file: &ports::ChangedFile<'_>,
    ) -> Result<moatless_types::pull_request_conversation::FileContentsPair, ports::PullRequestError>
    {
        unreachable!("no test in this file reads a bound pull request")
    }
    async fn create_review_guide_task(
        &self,
        _viewer: &ScopeContext,
        _workspace_id: &str,
        _name: &str,
        _message: String,
    ) -> Result<String, ports::PullRequestError> {
        unreachable!("no test in this file starts a guided review")
    }
    async fn fork_review_guide_task(
        &self,
        _viewer: &ScopeContext,
        _source_task_id: &str,
        _task_id: &str,
        _name: &str,
        _message: String,
    ) -> Result<bool, ports::PullRequestError> {
        unreachable!("no test in this file starts a guided review")
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
    ) -> Result<Vec<ports::UsageBucket>, moatless_types::store::StoreError> {
        unreachable!("no test in this file reads usage")
    }
}

/// Every viewer's settings in one map: the tests here sign in one viewer.
#[derive(Default)]
pub(super) struct MemorySettings(std::sync::Mutex<serde_json::Map<String, Value>>);

#[async_trait]
impl ports::ViewerSettingsStore for MemorySettings {
    async fn read(
        &self,
        _user_id: &uuid::Uuid,
    ) -> Result<serde_json::Map<String, Value>, moatless_types::store::StoreError> {
        #[allow(clippy::unwrap_used)]
        Ok(self.0.lock().unwrap().clone())
    }

    async fn merge(
        &self,
        _user_id: &uuid::Uuid,
        patch: serde_json::Map<String, Value>,
    ) -> Result<serde_json::Map<String, Value>, moatless_types::store::StoreError> {
        #[allow(clippy::unwrap_used)]
        let mut stored = self.0.lock().unwrap();
        stored.extend(patch);
        Ok(stored.clone())
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
        _only_if_unused: bool,
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
    async fn set_auto_settle(
        &self,
        _viewer: &ScopeContext,
        _task_id: &str,
        _enabled: bool,
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
pub(crate) fn state() -> UiRpcState {
    state_with_attachments(Arc::new(RecordingAttachments::default()))
}

pub(super) fn state_with_attachments(attachments: Arc<dyn ports::AttachmentStore>) -> UiRpcState {
    let mut conversations = stub_conversations();
    conversations
        .expect_list_followed()
        .returning(|_| Ok(Vec::new()));
    state_over(attachments, conversations)
}

/// [`state`] with every full listing failing as `error` builds it.
pub(super) fn state_with_listing_failing(error: fn() -> ListingError) -> UiRpcState {
    let mut conversations = stub_conversations();
    conversations
        .expect_list_followed()
        .returning(move |_| Err(error()));
    conversations
        .expect_list_followed_closed()
        .returning(move |_, _| Err(error()));
    state_over(Arc::new(RecordingAttachments::default()), conversations)
}

/// The stub [`state`] reads from, short of the full listings.
fn stub_conversations() -> MockConversationDao {
    let mut conversations = MockConversationDao::new();
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
        Ok(ListingHead {
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
    conversations
}

fn state_over(
    attachments: Arc<dyn ports::AttachmentStore>,
    conversations: MockConversationDao,
) -> UiRpcState {
    let mut workspaces = MockWorkspaceListing::new();
    workspaces
        .expect_list_readable_including()
        .returning(|_, _| Ok(Vec::new()));
    let conversations: Arc<dyn ConversationDao> = Arc::new(conversations);
    UiRpcState {
        rpc: Arc::new(UiRpcConfig {
            environment_id: environment_id(&stub_identity()),
            allowed_origins: vec![AllowedOrigin::parse("http://localhost:5733")],
            otlp_traces_enabled: false,
            attachment_uploads: true,
            usage_limit_sources: false,
            orchestration_protocol_version: 1,
            review_guide_fork_author: false,
            scratch_workspace: None,
        }),
        environment: Arc::new(stub_identity()),
        session_cookie_name: Arc::from("moatless_session"),
        frontend_url: Arc::from("http://localhost:3000"),
        session_jwt_secret: Some(Arc::from("test-secret")),
        pairing: Arc::new(test_pairing(None)),
        access_changes: crate::access::AccessChanges::new(),
        reads: T3Reads::new(
            conversations.clone(),
            Arc::new(workspaces),
            Arc::new(MockTaskReader::new()),
            crate::poll_gate::PollGate::new(4),
        ),
        changes: crate::change_watch::ChangeWatch::new(),
        sandboxes: Arc::new(UnreachableSandboxes),
        snapshots: None,
        scripts: ScriptsState {
            workspace_scripts: Arc::new(ports::MockWorkspaceScriptSource::new()),
            ports: Arc::new(UnreachableSandboxes),
            preview: PreviewState {
                tabs: Arc::new(preview_tabs::PreviewTabs::new(preview_tabs::PreviewTabTtl(
                    std::time::Duration::from_hours(1),
                ))),
                tasks: Arc::new(MockTaskReader::new()),
                relay: preview_relay::PreviewRelay::disabled(),
            },
        },
        sandbox: SandboxState {
            observer: Arc::new(UnreachableSandboxes),
            tasks: Arc::new(MockTaskReader::new()),
            intent: Arc::new(UnreachableSandboxes),
            starter: Arc::new(UnreachableSandboxes),
            stopper: Arc::new(UnreachableSandboxes),
            operator: Arc::new(UnreachableSandboxes),
            diagnostics: Arc::new(UnreachableSandboxes),
            poll_gate: crate::poll_gate::PollGate::new(4),
            events: Arc::new(crate::sandbox_setup::LoggedEvents::default()),
        },
        subtasks: SubtasksState {
            relations: Arc::new(NoChildTasks),
            conversations,
        },
        preview: PreviewState {
            tabs: Arc::new(preview_tabs::PreviewTabs::new(preview_tabs::PreviewTabTtl(
                std::time::Duration::from_hours(1),
            ))),
            tasks: Arc::new(MockTaskReader::new()),
            relay: preview_relay::PreviewRelay::disabled(),
        },
        agents: Arc::new(StubAgents),
        branches: Arc::new(UnreachableBranches),
        skills: Arc::new(StubSkills),
        pull_requests: Arc::new(NoBoundPullRequests),
        pull_request_reviews: Arc::new(UnreachablePullRequestReviews),
        review_guides: moatless_tasks::compose::task_graph(
            sqlx::PgPool::connect_lazy("postgres://unused").expect("a lazy pool"),
        )
        .review_guides(),
        source_control_auth: Arc::new(NoGithubIdentity),
        commands: Arc::new(UnreachableCommands),
        attachments,
        artifacts: Arc::new(ports::MockArtifactVisibilitySetter::new()),
        secret_requests: Arc::new(ports::MockSecretRequestAnswerer::new()),
        usage: Arc::new(UnreachableUsage),
        viewer_settings: Arc::new(MemorySettings::default()),
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
/// [`connection`] for tests on a [`Handler::Detached`](crate::rpc::table::Handler) method.
pub(super) fn session() -> ProtocolSession {
    ProtocolSession::new(crate::rpc::outbound::channel(OUTBOUND_FRAMES).0, methods())
}

/// A session holding the frame channel `serve_websocket` writes to the socket.
pub(super) struct Connection {
    pub(super) session: ProtocolSession,
    pushed: tokio::sync::mpsc::Receiver<crate::rpc::outbound::QueuedFrame>,
}

pub(super) fn connection() -> Connection {
    let (frames, pushed) = crate::rpc::outbound::channel(OUTBOUND_FRAMES);
    Connection {
        session: ProtocolSession::new(frames, methods()),
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
        self.pushed
            .try_recv()
            .ok()
            .map(crate::rpc::outbound::QueuedFrame::into_value)
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
            .into_value()
    }
}

/// A Task projection that answers for `readable` and for nothing else, which is
/// what a scoped read does for a viewer without access.
pub(super) fn tasks_readable(readable: &'static str) -> Arc<dyn moatless_ports::task::TaskReader> {
    let mut tasks = MockTaskReader::new();
    tasks
        .expect_describe()
        .returning(move |task_id, _| Ok((task_id == readable).then(|| super::open_task(task_id))));
    tasks
        .expect_may_access()
        .returning(move |task_id, _| Ok(task_id == readable));
    Arc::new(tasks)
}

/// `owned` is owned and readable, `visited` only readable: a public Task's
/// visitor.
pub(super) fn tasks_owned_and_visited(
    owned: &'static str,
    visited: &'static str,
) -> Arc<dyn moatless_ports::task::TaskReader> {
    let mut tasks = MockTaskReader::new();
    tasks
        .expect_may_access()
        .returning(move |task_id, _| Ok(task_id == owned || task_id == visited));
    tasks
        .expect_owned()
        .returning(move |task_id, _| Ok((task_id == owned).then(|| super::open_task(task_id))));
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

pub(crate) fn viewer() -> UiRpcAuthIdentity {
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
