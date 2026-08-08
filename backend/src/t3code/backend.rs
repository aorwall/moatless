//! The Moatless side of the compatibility module's write ports.
//!
//! One implementation each of [`AgentCatalogSource`] and [`ThreadCommandSink`],
//! over [`AppState`]. Everything here is translation: a T3 command has already
//! been validated by `super::commands` by the time it arrives, and each method
//! turns it into the same service call the Moatless UI makes.

use std::sync::Arc;

use async_trait::async_trait;
use moatless_api_schemas::agent::AgentDescriptor;
use moatless_backend_core::scope::ScopeContext;

use crate::adapters::models::TaskBinding;
use crate::file_storage::store as file_store;
use crate::message::error::MessageError;
use crate::message::service as message_service;
use crate::repository::error::RepositoryError;
use crate::repository::handlers::list_branches;
use crate::state::AppState;
use crate::task::dao::TaskPatch;
use crate::task::error::TaskError;
use crate::task::handlers as task_handlers;
use crate::task::models::TaskStatus;
use crate::task::service as task_service;
use moatless_t3code::ports::{
    AgentCatalogSource, BoundPullRequest, BoundPullRequestSource, BranchListError, CommandRefusal,
    GithubIdentity, NewThread, PullRequestError, QuestionAnswer, RepositoryBranchSource,
    ResolvedPullRequest, SourceControlAuthSource, ThreadCommandSink, ThreadMeta, ThreadTurn,
    TurnAttachment,
};

pub struct AppStateAgents(pub AppState);

#[async_trait]
impl AgentCatalogSource for AppStateAgents {
    async fn list(&self, viewer: &ScopeContext) -> Vec<AgentDescriptor> {
        crate::agent::service::list_agents(
            self.0.secret_records.as_ref(),
            self.0.provider_tokens.as_ref(),
            viewer,
        )
        .await
    }
}

pub struct AppStateBranches(pub AppState);

#[async_trait]
impl RepositoryBranchSource for AppStateBranches {
    async fn list_branches(
        &self,
        viewer: &ScopeContext,
        repository_id: &str,
    ) -> Result<moatless_api_schemas::repository::BranchListResponse, BranchListError> {
        list_branches(
            &self.0.repositories,
            &self.0.repo_token_resolver,
            &self.0.provider_tokens,
            viewer,
            repository_id,
        )
        .await
        .map_err(branch_list_error)
    }
}

/// A repository read failure, as the branch picker is told it.
///
/// A repository the viewer cannot read answers `NotFound`, the same as one that
/// is gone — neither is a branch list to show. A missing token or a git host
/// that refused is surfaced verbatim; a database failure is kept apart so it is
/// logged rather than sent.
fn branch_list_error(error: RepositoryError) -> BranchListError {
    match error {
        RepositoryError::NotFound(_) | RepositoryError::Forbidden(_) => BranchListError::None,
        RepositoryError::BadRequest(message)
        | RepositoryError::ProviderError(message)
        | RepositoryError::Conflict(message) => BranchListError::Unavailable(message),
        RepositoryError::Db(error) => BranchListError::Failed(error),
    }
}

pub struct AppStateBoundPullRequests(pub AppState);

#[async_trait]
impl BoundPullRequestSource for AppStateBoundPullRequests {
    async fn bound_pull_request(&self, task_id: &str) -> Option<BoundPullRequest> {
        let bindings = self.0.task_bindings.find_by_task_id(task_id).await.ok()?;
        // The active GitHub PR binding, if any. A Task has at most one.
        let binding = bindings.into_iter().find(|binding| {
            binding.active
                && binding.adapter_kind == "github"
                && binding.subject_kind == "github_pr"
        })?;
        if let Some(pr) = bound_pull_request_from_binding(&binding) {
            return Some(pr);
        }
        // The persisted blob predates base and head, so it yields no reportable
        // pull request. This is the only path that reads the binding — the
        // task-connections endpoint's stale-status refresh never runs here — so
        // refresh it once through the same write-through, then rebuild from what
        // the fetch filled in. A refresh that cannot be done (no token, API
        // error, no adapter) leaves the toolbar showing no pull request.
        let refreshed = self.refresh_incomplete_binding(task_id, &binding).await?;
        bound_pull_request_from_binding(&refreshed)
    }

    async fn resolve_pull_request(
        &self,
        viewer: &ScopeContext,
        repository_id: &str,
        reference: &str,
    ) -> Result<ResolvedPullRequest, PullRequestError> {
        let (owner, repo) = self.github_owner_repo(viewer, repository_id).await?;
        crate::adapters::providers::github_pr::api_routes::resolve_reference_for_dialog(
            &self.0,
            &viewer.user_id,
            &owner,
            &repo,
            reference,
        )
        .await
        .map_err(pull_request_error)?
        .map(resolved_pull_request)
        .ok_or(PullRequestError::NotFound)
    }

    async fn open_pull_request_for_head(
        &self,
        viewer: &ScopeContext,
        repository_id: &str,
        head: &str,
    ) -> Option<ResolvedPullRequest> {
        let (owner, repo) = self.github_owner_repo(viewer, repository_id).await.ok()?;
        crate::adapters::providers::github_pr::api_routes::find_open_pull_request_for_head(
            &self.0,
            &viewer.user_id,
            &owner,
            &repo,
            head,
        )
        .await
        .ok()?
        .map(resolved_pull_request)
    }
}

impl AppStateBoundPullRequests {
    /// Fetch live status for a binding whose persisted blob is incomplete and
    /// return a copy carrying the freshly written-through fields. `None` when no
    /// live status could be obtained.
    async fn refresh_incomplete_binding(
        &self,
        task_id: &str,
        binding: &TaskBinding,
    ) -> Option<TaskBinding> {
        let created_by = self.task_creator(task_id).await?;
        let status = crate::bindings::status_service::get_or_fetch(
            self.0.adapters.as_ref(),
            self.0.task_bindings.as_ref(),
            binding,
            &created_by,
        )
        .await?;
        let mut refreshed = binding.clone();
        refreshed.external_status = status.status.clone();
        refreshed.external_state_kind = status.state_kind.clone();
        refreshed.external_status_raw = Some(status.raw.clone());
        Some(refreshed)
    }

    /// The login of the Task's creator, whose token the GitHub status fetch
    /// resolves. Read directly rather than through a scoped find because the
    /// port has already had the viewer's access to the Task checked.
    async fn task_creator(&self, task_id: &str) -> Option<String> {
        sqlx::query_scalar::<_, Option<String>>("SELECT created_by FROM tasks WHERE task_id = $1")
            .bind(task_id)
            .fetch_optional(&self.0.db)
            .await
            .ok()
            .flatten()
            .flatten()
    }

    /// The `owner/repo` a repository's GitHub remote names, if the viewer can be
    /// shown the repository and it is a GitHub remote.
    async fn github_owner_repo(
        &self,
        viewer: &ScopeContext,
        repository_id: &str,
    ) -> Result<(String, String), PullRequestError> {
        let repository = self
            .0
            .repositories
            .find_by_id(repository_id, viewer)
            .await
            .map_err(|error| PullRequestError::Failed(error.to_string()))?
            .ok_or(PullRequestError::NotFound)?;
        let remote_url = repository.remote_url.ok_or(PullRequestError::NotFound)?;
        crate::repository::access::parse_github_owner_repo(&remote_url).ok_or_else(|| {
            PullRequestError::Unavailable("This repository has no GitHub remote.".to_string())
        })
    }
}

/// A GitHub API failure, as the attach-PR dialog is told it.
///
/// A reference that names no pull request the credential can see is `NotFound`;
/// a missing credential or a host that refused is surfaced verbatim; anything
/// else is this deployment's own failure, logged rather than sent.
fn pull_request_error(error: crate::adapters::error::AdapterError) -> PullRequestError {
    use crate::adapters::error::{AdapterError, SharedAdapterError};
    match error {
        AdapterError::UserCredential { .. } => PullRequestError::Unavailable(
            "Connect a GitHub account to resolve pull requests.".to_string(),
        ),
        AdapterError::Shared(SharedAdapterError::AdapterApi(message)) => {
            PullRequestError::Unavailable(message)
        }
        other => PullRequestError::Failed(other.to_string()),
    }
}

fn resolved_pull_request(
    pr: crate::adapters::providers::github_pr::api_routes::DialogPullRequest,
) -> ResolvedPullRequest {
    ResolvedPullRequest {
        number: pr.number,
        title: pr.title,
        url: pr.url,
        base_branch: pr.base_ref,
        head_branch: pr.head_ref,
        state: pr.state,
    }
}

/// Bind a newly created Task to the open pull request whose head is its branch,
/// when the branch names one.
///
/// Best-effort: a thread whose branch has no open PR, whose repository has no
/// GitHub remote, or whose host cannot be reached is created unbound rather than
/// refused — creating the thread is the operation the person asked for, and the
/// binding is a convenience layered on it.
async fn bind_thread_to_open_pr(
    state: &AppState,
    viewer: &ScopeContext,
    workspace_id: &str,
    branch: &str,
    task_id: &str,
) {
    let Some(repository_id) = state
        .workspaces
        .find_primary_repo(workspace_id)
        .await
        .ok()
        .flatten()
        .map(|placement| placement.repository_id)
    else {
        return;
    };
    let Some(remote_url) = state
        .repositories
        .find_by_id(&repository_id, viewer)
        .await
        .ok()
        .flatten()
        .and_then(|repository| repository.remote_url)
    else {
        return;
    };
    let Some((owner, repo)) = crate::repository::access::parse_github_owner_repo(&remote_url)
    else {
        return;
    };
    let pr =
        match crate::adapters::providers::github_pr::api_routes::find_open_pull_request_for_head(
            state,
            &viewer.user_id,
            &owner,
            &repo,
            branch,
        )
        .await
        {
            Ok(Some(pr)) => pr,
            Ok(None) => return,
            Err(error) => {
                tracing::debug!(%error, task_id, branch, "T3 thread: no open PR for branch to bind");
                return;
            }
        };
    let source_id = format!("{owner}/{repo}");
    let thread_properties = serde_json::json!({
        "owner": owner,
        "repo": repo,
        "pr_number": pr.number,
    });
    match crate::loops::firing::listen::Connector::new(
        state.adapters.clone(),
        state.adapter_connections.clone(),
        state.loops.clone(),
        state.task_bindings.clone(),
    )
    .connect_github_binding(
        &source_id,
        task_id,
        thread_properties,
        Some(&viewer.user_id),
    )
    .await
    {
        Ok((_, binding)) => tracing::info!(
            task_id,
            pr_number = pr.number,
            binding_id = %binding.id,
            "T3 thread bound to the open pull request on its branch"
        ),
        Err(error) => tracing::warn!(
            %error,
            task_id,
            pr_number = pr.number,
            "T3 thread failed to bind to the open pull request on its branch"
        ),
    }
}

pub struct AppStateSourceControlAuth(pub AppState);

#[async_trait]
impl SourceControlAuthSource for AppStateSourceControlAuth {
    async fn github_identity(&self, viewer: &ScopeContext) -> Option<GithubIdentity> {
        let presence = self
            .0
            .provider_tokens
            .find_github_scope_presence(&viewer.user_id)
            .await
            .ok()??;
        // A row the refresher marked failed is a connection the viewer must
        // renew, so it is not a usable credential the panel should call
        // authenticated.
        if presence.refresh_failure {
            return None;
        }
        Some(GithubIdentity {
            account: presence.login,
        })
    }
}

/// Recover the pull request a GitHub binding points at, in the shape the status
/// surface reports. `None` when the persisted blob predates base and head, so
/// the toolbar shows no pull request rather than one with blank branches.
fn bound_pull_request_from_binding(binding: &TaskBinding) -> Option<BoundPullRequest> {
    let number = binding.subject_key.rsplit_once('#')?.1.parse().ok()?;
    let raw = binding.external_status_raw.as_ref()?;
    let non_empty = |key: &str| {
        raw.get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let base_ref = non_empty("baseRef")?;
    let head_ref = non_empty("headRef")?;
    let title = non_empty("title").unwrap_or_else(|| binding.subject_key.clone());
    let url = non_empty("url").unwrap_or_else(|| pull_request_url(&binding.subject_key, number));
    let state_kind = binding
        .external_state_kind
        .clone()
        .unwrap_or_else(|| "open".to_string());
    Some(BoundPullRequest {
        number,
        title,
        url,
        base_ref,
        head_ref,
        state_kind,
    })
}

/// `https://github.com/{owner}/{repo}/pull/{number}` from a `owner/repo#N`
/// subject key, for a binding whose blob recorded no url.
fn pull_request_url(subject_key: &str, number: u64) -> String {
    let repo = subject_key.split('#').next().unwrap_or(subject_key);
    format!("https://github.com/{repo}/pull/{number}")
}

pub struct AppStateCommands(pub AppState);

impl AppStateCommands {
    pub fn new(state: AppState) -> Arc<Self> {
        Arc::new(Self(state))
    }

    /// A viewer as the message and task services expect one.
    ///
    /// `is_agent_context` is false because every command on this socket comes
    /// from a person looking at a screen — which is what makes the auto-follow
    /// on send correct here.
    fn sender(viewer: &ScopeContext) -> message_service::Sender<'_> {
        message_service::Sender {
            user_id: &viewer.user_id,
            scope: viewer,
            is_agent_context: false,
        }
    }

    async fn ensure_followed_open(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
    ) -> Result<(), CommandRefusal> {
        let Some(task) = self.0.tasks.find_by_id(task_id, viewer).await.map_err(|error| {
            tracing::warn!(%error, task_id, "T3 settlement command could not resolve its thread");
            CommandRefusal::Failed {
                message: "Could not reach that thread.".to_string(),
            }
        })?
        else {
            return Err(CommandRefusal::NotWritable {
                task_id: task_id.to_string(),
            });
        };
        if task.status != TaskStatus::Open {
            return Err(CommandRefusal::NotWritable {
                task_id: task_id.to_string(),
            });
        }
        let followed = self
            .0
            .tasks
            .get_followed(&viewer.user_id, &[task_id.to_string()])
            .await
            .map_err(|error| {
                tracing::warn!(%error, task_id, "T3 settlement command could not resolve follow");
                CommandRefusal::Failed {
                    message: "Could not reach that thread.".to_string(),
                }
            })?;
        if !followed.contains(task_id) {
            return Err(CommandRefusal::NotWritable {
                task_id: task_id.to_string(),
            });
        }
        Ok(())
    }

    fn state_refusal(task_id: &str, error: sqlx::Error) -> CommandRefusal {
        tracing::warn!(%error, task_id, "T3 settlement state update failed");
        CommandRefusal::Failed {
            message: "Could not update that thread.".to_string(),
        }
    }

    /// Whether this turn's Message is already persisted — the frame is a
    /// replay. The send path makes the same read and is what actually resolves
    /// the replay, so a lookup that fails answers `false` and leaves that
    /// decision where it belongs rather than refusing a turn over it.
    async fn already_sent(&self, turn: &ThreadTurn) -> bool {
        match self.0.messages.find_by_uuid(&turn.message_uuid).await {
            Ok(existing) => existing.is_some_and(|message| message.task_id == turn.task_id),
            Err(error) => {
                tracing::warn!(%error, task_id = %turn.task_id, "T3 turn could not be checked for replay");
                false
            }
        }
    }

    /// Store a turn's inline images as ordinary Moatless uploads, so they land
    /// on the paths that already exist: the Sandbox pre-fetch, the prompt block
    /// naming each file, the Moatless UI, and `GET /api/v1/uploads/{file_id}`.
    ///
    /// A failure refuses the whole turn — a partial store would leave a message
    /// naming only some of what was attached.
    async fn store_attachments(
        &self,
        viewer: &ScopeContext,
        attachments: &[TurnAttachment],
    ) -> Result<Vec<String>, CommandRefusal> {
        let mut file_ids = Vec::with_capacity(attachments.len());
        for attachment in attachments {
            let stored = file_store::store_upload(
                self.0.s3.as_ref(),
                &self.0.s3_config,
                self.0.files.as_ref(),
                viewer,
                file_store::NewUpload {
                    filename: attachment.filename.clone(),
                    mime_type: attachment.mime_type.clone(),
                    data: attachment.bytes.clone(),
                },
            )
            .await
            .map_err(|error| {
                tracing::warn!(%error, filename = %attachment.filename, "T3 attachment could not be stored");
                CommandRefusal::Failed {
                    message: format!("Could not store the attachment `{}`.", attachment.filename),
                }
            })?;
            file_ids.push(stored.file_id);
        }
        Ok(file_ids)
    }
}

#[async_trait]
impl ThreadCommandSink for AppStateCommands {
    async fn send_turn(
        &self,
        viewer: &ScopeContext,
        turn: ThreadTurn,
    ) -> Result<(), CommandRefusal> {
        // Stored before the Message, so a turn that reaches the agent always
        // names files that exist.
        //
        // A replay has to be caught here rather than left to the send path: the
        // send resolves it by reading the Message already under that uuid and
        // writing nothing, but by then these bytes are in S3 under a file id no
        // Message will ever name, and a reconnecting client replays whole
        // turns. The extra read only happens on turns that carry files.
        let file_ids = if turn.attachments.is_empty() || self.already_sent(&turn).await {
            Vec::new()
        } else {
            self.store_attachments(viewer, &turn.attachments).await?
        };

        message_service::send_user_message(
            self.0.tasks.as_ref(),
            self.0.message_send.as_ref(),
            self.0.sandbox_waker.as_ref(),
            &Self::sender(viewer),
            &turn.task_id,
            message_service::UserMessage {
                text: turn.text,
                model: turn.model,
                agent_mode: turn.agent_mode,
                effort: turn.effort,
                resume: true,
                message_uuid: Some(turn.message_uuid),
                file_ids,
                ..Default::default()
            },
        )
        .await
        .map(|_| ())
        .map_err(|e| send_refusal(&turn.task_id, e))
    }

    async fn create_thread(
        &self,
        viewer: &ScopeContext,
        thread: NewThread,
    ) -> Result<(), CommandRefusal> {
        // The transport is not selectable from T3, so a thread created there
        // runs the first catalog agent of the kind the person picked.
        let agent_type = default_agent_for(&thread.instance);

        // Kept past the request, which moves both: a thread created on a PR's
        // head branch is bound to that PR once the Task exists.
        let workspace_id = thread.workspace_id.clone();
        let branch = thread.branch.clone();

        let file_ids = match thread.first_message.as_ref() {
            Some(first) => self.store_attachments(viewer, &first.attachments).await?,
            None => Vec::new(),
        };

        let request = moatless_api_schemas::task::CreateTaskRequest {
            repository_id: None,
            workspace_id: Some(thread.workspace_id),
            agent_type: Some(agent_type),
            name: thread.title,
            description: None,
            branch: thread.branch,
            message: thread.first_message.as_ref().map(|m| m.text.clone()),
            model: thread.model,
            agent_mode: thread
                .first_message
                .as_ref()
                .and_then(|m| m.agent_mode.clone()),
            effort: thread.first_message.as_ref().and_then(|m| m.effort.clone()),
            contexts: vec![],
            file_ids,
            parent_task_id: None,
            headless: Some(false),
        };

        let mut intent = task_service::CreateTaskIntent::new(request);
        intent.task_id = Some(thread.task_id.clone());
        // A thread created with nothing in it gets no sandbox until something
        // is sent to it.
        intent.provision = thread.first_message.is_some();
        // T3's optimistic copy of the first message is keyed on the id it
        // minted, so that id has to be the one persisted.
        intent.first_message_uuid = thread.first_message.map(|m| m.message_uuid);

        let new_task = task_service::normalize(
            self.0.workspaces.as_ref(),
            self.0.repositories.as_ref(),
            &self.0.credential_sync,
            &self.0.config,
            viewer,
            &viewer.user_id,
            intent,
        )
        .await
        .map_err(|e| create_refusal(&thread.task_id, e))?;
        task_service::create_task(
            &self.0.task_creation,
            self.0.message_send.clone(),
            new_task,
            |_| async { serde_json::Value::Null },
        )
        .await
        .map_err(|e| create_refusal(&thread.task_id, e))?;

        // The PR-prepared thread flow drops the pull-request reference before
        // `thread.create` (ChatView.tsx), so the head branch is the only signal
        // that this thread belongs to a pull request. Bind it here from that
        // branch so a review comment on the PR is delivered into this Task.
        if let Some(branch) = branch.as_deref().filter(|branch| !branch.is_empty()) {
            bind_thread_to_open_pr(&self.0, viewer, &workspace_id, branch, &thread.task_id).await;
        }
        Ok(())
    }

    async fn update_meta(
        &self,
        viewer: &ScopeContext,
        meta: ThreadMeta,
    ) -> Result<(), CommandRefusal> {
        if meta.title.is_none() && meta.model.is_none() {
            return Ok(());
        }
        let patch = TaskPatch {
            name: meta.title,
            model: meta.model,
            ..Default::default()
        };
        let updated = self
            .0
            .tasks
            .update_task(&meta.task_id, &patch, viewer)
            .await
            .map_err(|error| {
                tracing::warn!(%error, task_id = %meta.task_id, "T3 meta update failed");
                CommandRefusal::Failed {
                    message: "Could not update that thread.".to_string(),
                }
            })?;
        // `update_task` carries the access rule in its `WHERE`, so a viewer who
        // may not control the Task gets a clean `false` rather than an error.
        // Reporting that as success would leave T3 showing a title nothing
        // stored.
        if !updated {
            return Err(CommandRefusal::NotWritable {
                task_id: meta.task_id,
            });
        }
        Ok(())
    }

    async fn stop(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        task_service::stop_task(
            self.0.tasks.as_ref(),
            self.0.sandbox.as_ref(),
            self.0.events.as_deref(),
            viewer,
            task_id,
        )
        .await
        .map_err(|e| create_refusal(task_id, e))
    }

    async fn archive(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.0
            .task_closer
            .close_for(viewer, task_id)
            .await
            .map_err(|e| create_refusal(task_id, e))
    }

    async fn unarchive(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        task_handlers::reopen_task_for(
            self.0.tasks.as_ref(),
            self.0.events.as_deref(),
            viewer,
            task_id,
        )
        .await
        .map_err(|e| create_refusal(task_id, e))
    }

    async fn settle(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.ensure_followed_open(viewer, task_id).await?;
        self.0
            .tasks
            .settle_task(&viewer.user_id, task_id, chrono::Utc::now())
            .await
            .map_err(|e| Self::state_refusal(task_id, e))?;
        // Settling says nobody is waiting on this thread any more, so the
        // sandbox it was holding is released rather than left for the idle
        // reaper. Best-effort: the settlement is already durable, and a viewer
        // who follows a thread they may not control has no sandbox to stop —
        // neither case is a reason to report the settle as refused. New
        // activity clears the settlement (`clear_settled_for_all_followers`)
        // and the next turn starts the sandbox again.
        if let Err(error) = task_service::stop_task(
            self.0.tasks.as_ref(),
            self.0.sandbox.as_ref(),
            self.0.events.as_deref(),
            viewer,
            task_id,
        )
        .await
        {
            tracing::warn!(%error, task_id, "Sandbox stop on settle failed");
        }
        Ok(())
    }

    async fn unsettle(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.ensure_followed_open(viewer, task_id).await?;
        self.0
            .tasks
            .unsettle_task(&viewer.user_id, task_id)
            .await
            .map_err(|e| Self::state_refusal(task_id, e))
    }

    async fn snooze(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        snoozed_until: chrono::DateTime<chrono::Utc>,
    ) -> Result<(), CommandRefusal> {
        self.ensure_followed_open(viewer, task_id).await?;
        self.0
            .tasks
            .snooze_task(&viewer.user_id, task_id, snoozed_until, chrono::Utc::now())
            .await
            .map_err(|e| Self::state_refusal(task_id, e))
    }

    async fn unsnooze(&self, viewer: &ScopeContext, task_id: &str) -> Result<(), CommandRefusal> {
        self.ensure_followed_open(viewer, task_id).await?;
        self.0
            .tasks
            .unsnooze_task(&viewer.user_id, task_id)
            .await
            .map_err(|e| Self::state_refusal(task_id, e))
    }

    async fn answer_question(
        &self,
        viewer: &ScopeContext,
        answer: QuestionAnswer,
    ) -> Result<(), CommandRefusal> {
        message_service::send_user_message(
            self.0.tasks.as_ref(),
            self.0.message_send.as_ref(),
            self.0.sandbox_waker.as_ref(),
            &Self::sender(viewer),
            &answer.task_id,
            message_service::UserMessage {
                resume: true,
                tool_response: Some(message_service::ToolResponse {
                    tool_use_id: answer.tool_use_id,
                    tool_name: answer.tool_name,
                    payload: answer.payload,
                }),
                ..Default::default()
            },
        )
        .await
        .map(|_| ())
        .map_err(|e| send_refusal(&answer.task_id, e))
    }

    async fn set_branch(
        &self,
        viewer: &ScopeContext,
        task_id: &str,
        branch: &str,
    ) -> Result<(), CommandRefusal> {
        let patch = TaskPatch {
            branch: Some(branch.to_string()),
            ..Default::default()
        };
        let updated = self
            .0
            .tasks
            .update_task(task_id, &patch, viewer)
            .await
            .map_err(|error| {
                tracing::warn!(%error, task_id, "T3 branch update failed");
                CommandRefusal::Failed {
                    message: "Could not record the branch on that thread.".to_string(),
                }
            })?;
        // The control filter is in the `WHERE`, so a viewer who may not control
        // the Task gets a clean `false` rather than an error.
        if !updated {
            return Err(CommandRefusal::NotWritable {
                task_id: task_id.to_string(),
            });
        }
        Ok(())
    }
}

/// The agent a thread created from T3 runs.
fn default_agent_for(instance: &str) -> moatless_core::shared_models::AgentType {
    let id = crate::agent::catalog::catalog()
        .iter()
        .find(|entry| agent_instance_for(entry.kind) == instance)
        .map_or(
            moatless_core::shared_models::AgentType::CLAUDE_CODE,
            |entry| entry.id,
        );
    moatless_core::shared_models::AgentType(id.to_string())
}

fn agent_instance_for(kind: crate::agent::catalog::AgentKind) -> &'static str {
    match kind {
        crate::agent::catalog::AgentKind::ClaudeCode => "claudeAgent",
        crate::agent::catalog::AgentKind::Codex => "codex",
    }
}

fn send_refusal(task_id: &str, error: MessageError) -> CommandRefusal {
    match error {
        MessageError::NotFound(_) => CommandRefusal::NotWritable {
            task_id: task_id.to_string(),
        },
        MessageError::TaskClosed(_) => CommandRefusal::Rejected {
            message: "This task is closed and cannot take new messages.".to_string(),
        },
        MessageError::BadRequest(message) => CommandRefusal::Rejected { message },
        other => {
            tracing::warn!(error = %other, task_id, "T3 send failed");
            CommandRefusal::Failed {
                message: "The message could not be sent.".to_string(),
            }
        }
    }
}

fn create_refusal(task_id: &str, error: TaskError) -> CommandRefusal {
    match error {
        TaskError::NotFound(_) | TaskError::Forbidden(_) => CommandRefusal::NotWritable {
            task_id: task_id.to_string(),
        },
        TaskError::BadRequest(message) => CommandRefusal::Rejected { message },
        TaskError::RepoNotFound(id) => CommandRefusal::Rejected {
            message: format!("Repository {id} could not be found."),
        },
        other => {
            tracing::warn!(error = %other, task_id, "T3 task command failed");
            CommandRefusal::Failed {
                message: "The action could not be completed.".to_string(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::catalog;
    /// The instance a person picks in T3 is what decides which agent runs the
    /// thread. Getting this backwards creates a Claude Task for somebody who
    /// asked for Codex, and the harness cannot be changed afterwards.
    #[test]
    fn each_instance_maps_to_the_first_catalog_agent_of_its_kind() {
        assert_eq!(
            default_agent_for("claudeAgent"),
            moatless_core::shared_models::AgentType(catalog::CLAUDE_CODE.id.to_string())
        );
        assert_eq!(
            default_agent_for("codex"),
            moatless_core::shared_models::AgentType(catalog::CODEX.id.to_string())
        );
    }

    /// An instance this server never published cannot arrive from a client that
    /// read our provider list, so it is a bug rather than a choice. Claude is
    /// the safe floor: it is the deployment's default harness.
    #[test]
    fn an_unknown_instance_falls_back_to_claude() {
        assert_eq!(
            default_agent_for("somethingElse"),
            moatless_core::shared_models::AgentType(
                moatless_core::shared_models::AgentType::CLAUDE_CODE.to_string()
            )
        );
    }

    /// The attach-PR dialog reads two `AdapterError` arms by name; the rest fall
    /// through a wildcard. Pin those two so a later rename or descent of the
    /// adapter error surface cannot silently reclassify a recognised failure as
    /// this deployment's own bug.
    #[test]
    fn the_two_recognised_adapter_errors_stay_actionable() {
        use crate::adapters::error::{AdapterError, SharedAdapterError};

        let missing_credential = pull_request_error(AdapterError::UserCredential {
            user_id: "u-1".into(),
            source: crate::auth::error::AuthError::Forbidden,
        });
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
}
