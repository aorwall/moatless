//! The Moatless side of the compatibility module's write ports.
//!
//! One implementation each of [`AgentCatalogSource`] and [`ThreadCommandSink`],
//! over [`AppState`]. Everything here is translation: a T3 command has already
//! been validated by `super::commands` by the time it arrives, and each method
//! turns it into the same service call the Moatless UI makes.

use std::sync::Arc;

use async_trait::async_trait;
use moatless_api_schemas::agent::AgentDescriptor;
use moatless_types::access_control::{ScopeContext, ScopedResource};

use crate::file_storage::error::FileStorageError;
use moatless_adapter_model::models::TaskBinding;
use crate::file_storage::store as file_store;
use crate::message::error::MessageError;
use crate::message::service as message_service;
use crate::repository::error::RepositoryError;
use crate::repository::handlers::list_branches;
use crate::state::AppState;
use crate::task::dao::{TaskDao, TaskPatch};
use crate::task::error::TaskError;
use crate::task::handlers as task_handlers;
use crate::task::models::TaskStatus;
use crate::task::service as task_service;
use moatless_t3code::ports::{
    AgentCatalogSource, AttachmentKind, AttachmentStore, AttachmentStoreError, BoundPullRequest,
    BoundPullRequestSource, BranchListError, CommandRefusal, DeclaredAttachment, GithubIdentity,
    NewThread, PullRequestError, QuestionAnswer, RepositoryBranchSource, ResolvedPullRequest,
    SourceControlAuthSource, ThreadCommandSink, ThreadMeta, ThreadTurn, TurnAttachment,
};

pub struct AppStateAgents(pub AppState);

#[async_trait]
impl AgentCatalogSource for AppStateAgents {
    async fn list(&self, viewer: &ScopeContext) -> Vec<AgentDescriptor> {
        crate::agent::service::list_agents(
            self.0.secrets.as_ref(),
            self.0.harness_credentials.as_ref(),
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
        workspace_id: Option<&str>,
    ) -> Result<moatless_api_schemas::repository::BranchListResponse, BranchListError> {
        list_branches(
            &self.0.repositories,
            &self.0.workspaces,
            &self.0.git_host_tokens,
            &self.0.provider_tokens,
            viewer,
            repository_id,
            workspace_id,
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
        RepositoryError::Db(error) => BranchListError::Failed(
            moatless_backend_core::store::StoreError::with_source("list repository branches", error),
        ),
        RepositoryError::Store(error) => BranchListError::Failed(error),
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
                && binding.adapter_kind == moatless_types::adapter::AdapterKind::Github
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
        let adapter = self
            .0
            .adapter_installations
            .as_ref()
            .map(|installations| installations.github_pr())
            .ok_or(PullRequestError::NotFound)?;
        // The dialog resolves a reference against a Repository the viewer
        // names, with no Task and so no Workspace whose credential could stand
        // in for the viewer's own.
        crate::adapters::providers::github_pr::api_routes::resolve_reference_for_dialog(
            &adapter,
            &viewer.user_id,
            None,
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
        let adapter = self
            .0
            .adapter_installations
            .as_ref()
            .map(|installations| installations.github_pr())?;
        crate::adapters::providers::github_pr::api_routes::find_open_pull_request_for_head(
            &adapter,
            &viewer.user_id,
            None,
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
        moatless_types::repository::GitHostRepo::parse(
            moatless_types::repository::RepositoryProvider::Github,
            &remote_url,
        )
        .map(|repo| (repo.owner().to_string(), repo.name().to_string()))
        .ok_or_else(|| {
            PullRequestError::Unavailable("This repository has no GitHub remote.".to_string())
        })
    }
}

/// A GitHub API failure, as the attach-PR dialog is told it.
///
/// A reference that names no pull request the credential can see is `NotFound`;
/// a missing credential or a host that refused is surfaced verbatim; anything
/// else is this deployment's own failure, logged rather than sent.
fn pull_request_error(error: moatless_adapter_model::error::AdapterError) -> PullRequestError {
    use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
    match error {
        AdapterError::UpstreamCredential { .. } => PullRequestError::Unavailable(
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
    let Some(parsed) = moatless_types::repository::GitHostRepo::parse(
        moatless_types::repository::RepositoryProvider::Github,
        &remote_url,
    ) else {
        return;
    };
    let (owner, repo) = (parsed.owner(), parsed.name());
    let Some(adapter) = state
        .adapter_installations
        .as_ref()
        .map(|installations| installations.github_pr())
    else {
        return;
    };
    let pr =
        match crate::adapters::providers::github_pr::api_routes::find_open_pull_request_for_head(
            &adapter,
            &viewer.user_id,
            Some(workspace_id),
            owner,
            repo,
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
        state
            .adapter_installations
            .as_ref()
            .map(|installations| installations.github_destination_builder()),
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

/// A turn's attachments as upload ids, and which of those this call stored.
struct ResolvedAttachments {
    /// Every id the Message will name, in the order they were attached.
    file_ids: Vec<String>,
    /// The subset stored here from inline bytes, and so the only ids a failed
    /// command may take back. An `Uploaded` id was put there by the client
    /// through `attachments.createUploadUrl`; the client still holds it and
    /// re-sends it on a retry, so deleting it would break the retry it exists
    /// for.
    stored_here: Vec<String>,
}

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

    /// The upload ids a turn's attachments name, storing the ones that arrived
    /// as bytes.
    ///
    /// Both forms end at the same place: a Moatless upload id on the Message,
    /// which is what the Sandbox pre-fetch, the prompt block naming each file,
    /// the Moatless UI and `GET /api/v1/uploads/{file_id}` all already read. An
    /// inline image is stored here because this is where its bytes enter; an
    /// uploaded attachment is only checked, because storing it again would put
    /// a second copy of the same file under a second id.
    ///
    /// A failure refuses the whole turn — a partial resolution would leave a
    /// message naming only some of what was attached — and takes back the
    /// inline bytes it had already stored.
    async fn resolve_attachments(
        &self,
        viewer: &ScopeContext,
        attachments: &[TurnAttachment],
    ) -> Result<ResolvedAttachments, CommandRefusal> {
        let mut resolved = ResolvedAttachments {
            file_ids: Vec::with_capacity(attachments.len()),
            stored_here: Vec::new(),
        };
        for attachment in attachments {
            let outcome = match attachment {
                TurnAttachment::Inline {
                    filename,
                    mime_type,
                    bytes,
                } => file_store::store_upload(
                    self.0.s3.as_ref(),
                    &self.0.s3_config,
                    self.0.files.as_ref(),
                    viewer,
                    file_store::NewUpload {
                        filename: filename.clone(),
                        mime_type: mime_type.clone(),
                        data: bytes.clone(),
                        file_id: None,
                    },
                )
                .await
                .map_err(|error| {
                    tracing::warn!(%error, filename = %filename, "T3 attachment could not be stored");
                    CommandRefusal::Failed {
                        message: format!("Could not store the attachment `{filename}`."),
                    }
                })
                .map(|stored| {
                    resolved.stored_here.push(stored.file_id.clone());
                    stored.file_id
                }),
                TurnAttachment::Uploaded {
                    attachment_id,
                    kind,
                } => self.uploaded_attachment(viewer, attachment_id, *kind).await,
            };
            match outcome {
                Ok(file_id) => resolved.file_ids.push(file_id),
                Err(refusal) => {
                    self.discard_attachments(viewer, &resolved.stored_here).await;
                    return Err(refusal);
                }
            }
        }
        Ok(resolved)
    }

    /// The upload an attachment id names, if this viewer may read it.
    ///
    /// Checked rather than trusted: the id reaches this server on a turn, and
    /// nothing about a turn proves the person sending it minted that id. An id
    /// that names nothing is the same answer as one that names somebody else's
    /// upload — a turn that could confirm the difference would be a way to ask
    /// whether an upload exists.
    async fn uploaded_attachment(
        &self,
        viewer: &ScopeContext,
        attachment_id: &str,
        kind: AttachmentKind,
    ) -> Result<String, CommandRefusal> {
        let found = self
            .0
            .files
            .find_by_id(attachment_id)
            .await
            .map_err(|error| {
                tracing::warn!(%error, attachment_id, "T3 attachment could not be resolved");
                CommandRefusal::Failed {
                    message: "Could not reach that attachment.".to_string(),
                }
            })?;
        let unreachable = || CommandRefusal::Rejected {
            message: "An attachment on this message is no longer available.".to_string(),
        };
        let Some(meta) = found else {
            return Err(unreachable());
        };
        let resource = ScopedResource::borrowed(
            meta.scope,
            meta.owner_user_id.as_deref().or(meta.created_by.as_deref()),
        );
        if !viewer.can_read(&resource) {
            tracing::warn!(attachment_id, "T3 turn named an attachment its sender cannot read");
            return Err(unreachable());
        }
        // Read access is not provenance. Moatless's own multipart route stores
        // files this viewer can read and knows nothing about T3's union, so
        // without this the mime and size limits would hold only for uploads
        // that actually came through the mint. It also keeps the turn agreeing
        // with the projection, which drops an attachment outside these bounds:
        // storing one would show the person a message whose attachment the
        // agent saw and their client never renders.
        if !kind.accepts(&meta.mime_type, meta.size) {
            tracing::warn!(
                attachment_id,
                mime_type = %meta.mime_type,
                size = meta.size,
                ?kind,
                "T3 turn named an attachment outside the member it claimed"
            );
            return Err(CommandRefusal::Rejected {
                message: format!(
                    "Attachment `{}` is not something that can be sent here.",
                    meta.filename
                ),
            });
        }
        Ok(meta.file_id)
    }

    /// Take back inline uploads a command that failed left behind.
    ///
    /// Only ever passed [`ResolvedAttachments::stored_here`]: the bytes reach
    /// S3 before the Message that would name them exists, so a command that is
    /// refused afterwards leaves objects nothing references and no route
    /// reaches.
    ///
    /// That is also why this needs no
    /// [`AppStateAttachments::is_on_a_message`] check, which
    /// `attachments.delete` and a re-`POST` both make. Those act on an id the
    /// client chose, at a time when a turn may already have been sent under it.
    /// These ids were minted inside the command that is now failing, and the
    /// command failing is precisely what means no Message was written — so no
    /// `task_message_contexts` row can name one.
    ///
    /// Best-effort by design: the caller is already returning a refusal, and a
    /// store that cannot be undone is a leak to notice in the logs rather than
    /// a second failure to put in front of the person retrying.
    async fn discard_attachments(&self, viewer: &ScopeContext, file_ids: &[String]) {
        for file_id in file_ids {
            if let Err(error) = file_store::delete_upload(
                self.0.s3.as_ref(),
                &self.0.s3_config,
                self.0.files.as_ref(),
                viewer,
                file_id,
            )
            .await
            {
                tracing::warn!(%error, file_id, "T3 attachment outlived the command it was sent with");
            }
        }
    }
}

/// Where a T3 chat attachment's bytes go: an ordinary Moatless upload, under
/// the id `attachments.createUploadUrl` already handed the client.
pub struct AppStateAttachments(pub AppState);

impl AppStateAttachments {
    /// Whether a sent message already carries this upload.
    ///
    /// The check lives here rather than in [`file_store::delete_upload`]: that
    /// helper is generic file storage and knows nothing about messages, while
    /// the rule — an attachment stops being the composer's draft the moment a
    /// turn names it — is this surface's.
    async fn is_on_a_message(&self, attachment_id: &str) -> Result<bool, AttachmentStoreError> {
        sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (
                SELECT 1 FROM task_message_contexts
                WHERE reference_id = $1 AND type = 'file'
             )",
        )
        .bind(attachment_id)
        .fetch_one(&self.0.db)
        .await
        .map_err(|error| {
            // Not knowing is not permission to delete: a read that failed is
            // reported as a failure rather than assumed to mean "unreferenced".
            tracing::warn!(%error, attachment_id, "Could not tell whether a T3 attachment is in use");
            AttachmentStoreError::Failed(error.to_string())
        })
    }
}

#[async_trait]
impl AttachmentStore for AppStateAttachments {
    async fn store(
        &self,
        viewer: &ScopeContext,
        attachment_id: &str,
        declared: DeclaredAttachment,
        bytes: Vec<u8>,
    ) -> Result<(), AttachmentStoreError> {
        // Once a turn carries this id, the bytes are what the transcript, the
        // download route and the Sandbox pre-fetch all point at, and replacing
        // them would change an attachment that has already been sent and read.
        // A retry cannot reach this: sending the turn required the first upload
        // to have finished.
        if self.is_on_a_message(attachment_id).await? {
            tracing::warn!(attachment_id, "Refused to replace a T3 attachment already on a message");
            return Err(AttachmentStoreError::Forbidden);
        }

        // A minted URL is good for ten minutes and a client that lost its
        // connection mid-upload may `POST` it again. Removing what the first
        // attempt left makes the second one land instead of colliding with it.
        // Nothing else can be under this id: it is a uuid minted for this
        // viewer and handed to nobody else.
        if let Err(error) = file_store::delete_upload(
            self.0.s3.as_ref(),
            &self.0.s3_config,
            self.0.files.as_ref(),
            viewer,
            attachment_id,
        )
        .await
        {
            tracing::debug!(%error, attachment_id, "T3 attachment had nothing to replace");
        }

        file_store::store_upload(
            self.0.s3.as_ref(),
            &self.0.s3_config,
            self.0.files.as_ref(),
            viewer,
            file_store::NewUpload {
                filename: declared.filename,
                mime_type: declared.mime_type,
                data: bytes,
                file_id: Some(attachment_id.to_string()),
            },
        )
        .await
        .map(|_| ())
        .map_err(attachment_store_error)
    }

    async fn delete(
        &self,
        viewer: &ScopeContext,
        attachment_id: &str,
    ) -> Result<(), AttachmentStoreError> {
        // `attachments.delete` means "I abandoned this upload". An upload a
        // message names was not abandoned, so there is no orphan to collect and
        // the request rests on a premise that is no longer true. Deleting
        // anyway would strip an attachment off a sent message and leave the
        // transcript, the download and the Sandbox pre-fetch pointing at bytes
        // that are gone. Reported as success because the client's intent —
        // leave nothing dangling — holds either way, and a failure here is not
        // something the person could act on.
        if self.is_on_a_message(attachment_id).await? {
            tracing::debug!(attachment_id, "Kept a T3 attachment that a message names");
            return Ok(());
        }

        file_store::delete_upload(
            self.0.s3.as_ref(),
            &self.0.s3_config,
            self.0.files.as_ref(),
            viewer,
            attachment_id,
        )
        .await
        .map(|_| ())
        .map_err(attachment_store_error)
    }
}

fn attachment_store_error(error: FileStorageError) -> AttachmentStoreError {
    match error {
        FileStorageError::Forbidden(_) => AttachmentStoreError::Forbidden,
        other => AttachmentStoreError::Failed(other.to_string()),
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
            self.resolve_attachments(viewer, &turn.attachments)
                .await?
                .file_ids
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
        // Before anything is stored: the id arrives from the client and may
        // still be held by a creation that failed, and the attachments below go
        // to S3 under a file id no Message would ever name if the create is
        // refused here.
        free_thread_id(self.0.tasks.as_ref(), viewer, &thread.task_id).await?;

        // The transport is not selectable from T3, so a thread created there
        // runs the first catalog agent of the kind the person picked.
        let agent_type = default_agent_for(&thread.instance);

        // Kept past the request, which moves both: a thread created on a PR's
        // head branch is bound to that PR once the Task exists.
        let workspace_id = thread.workspace_id.clone();
        let branch = thread.branch.clone();

        let attachments = match thread.first_message.as_ref() {
            Some(first) => self.resolve_attachments(viewer, &first.attachments).await?,
            None => ResolvedAttachments {
                file_ids: Vec::new(),
                stored_here: Vec::new(),
            },
        };

        let request = moatless_api_schemas::task::CreateTaskRequest {
            repository_id: None,
            workspace_id: Some(thread.workspace_id),
            sandbox_id: None,
            checkout_mode: None,
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
            file_ids: attachments.file_ids.clone(),
            parent_task_id: None,
            headless: Some(false),
            // T3 has no skill picker of its own.
            skills: vec![],
        };

        let mut intent = task_service::CreateTaskIntent::new(request);
        intent.task_id = Some(thread.task_id.clone());
        // A thread created with nothing in it gets no sandbox until something
        // is sent to it.
        intent.provision = thread.first_message.is_some();
        // T3's optimistic copy of the first message is keyed on the id it
        // minted, so that id has to be the one persisted.
        intent.first_message_uuid = thread.first_message.map(|m| m.message_uuid);

        // A create that fails writes no Message, so nothing will ever name the
        // inline bytes stored above — including the preflight failure this
        // thread id is being retried after, which would otherwise leave one
        // unreachable object per retry. Only what this call stored is taken
        // back; an uploaded attachment is the client's and outlives the retry.
        let new_task = match task_service::normalize(
            self.0.workspace_resolver.as_ref(),
            self.0.repository_reader.as_ref(),
            &self.0.credential_sync,
            &self.0.config,
            viewer,
            &viewer.user_id,
            intent,
        )
        .await
        {
            Ok(new_task) => new_task,
            Err(e) => {
                self.discard_attachments(viewer, &attachments.stored_here).await;
                return Err(create_refusal(&thread.task_id, e));
            }
        };
        if let Err(e) = task_service::create_task(
            &self.0.task_creation,
            self.0.message_send.clone(),
            new_task,
            |_| async { serde_json::Value::Null },
        )
        .await
        {
            self.discard_attachments(viewer, &attachments.stored_here).await;
            return Err(create_refusal(&thread.task_id, e));
        }

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
}

/// The agent a thread created from T3 runs.
fn default_agent_for(instance: &str) -> moatless_types::agent::AgentType {
    let id = crate::agent::catalog::catalog()
        .iter()
        .find(|entry| agent_instance_for(entry.kind) == instance)
        .map_or(
            moatless_types::agent::AgentType::CLAUDE_CODE,
            |entry| entry.id,
        );
    moatless_types::agent::AgentType(id.to_string())
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

/// Make the id a `thread.create` names available, or say why it is not.
///
/// T3's client mints the thread id and Moatless persists it verbatim, so an id
/// can arrive already taken by the Task a failed creation left behind: a
/// preflight failure persists the row as `Error` and *then* returns the error.
/// The client cannot escape that on its own — it rotates a draft id only on a
/// `thread.deleted` event, which this server never emits, and `thread.delete`
/// is refused — so without this the retry hits the unique index on
/// `tasks.task_id` and the draft is dead for as long as the person keeps it.
///
/// Only that incarnation is cleared, and the delete re-checks the condition in
/// its own `WHERE`: an id held by a Task that is alive or has a conversation in
/// it is refused, so `thread.create` never becomes a way to discard a thread.
/// The ownership check here is what keeps the refusal for somebody else's id
/// indistinguishable from an id this viewer may not read.
async fn free_thread_id(
    tasks: &dyn TaskDao,
    viewer: &ScopeContext,
    task_id: &str,
) -> Result<(), CommandRefusal> {
    let existing = tasks.find_by_id(task_id, viewer).await.map_err(|error| {
        tracing::warn!(%error, task_id, "T3 thread create could not read the id it was given");
        CommandRefusal::Failed {
            message: "The thread could not be created.".to_string(),
        }
    })?;
    // Free, or held by a Task this viewer cannot read. In the second case the
    // unique index refuses the create, which is the same answer without
    // confirming that the id is taken.
    let Some(existing) = existing else {
        return Ok(());
    };

    if !viewer.can_write(&ScopedResource::borrowed(
        existing.scope,
        existing
            .owner_user_id
            .as_deref()
            .or(existing.created_by.as_deref()),
    )) {
        return Err(CommandRefusal::NotWritable {
            task_id: task_id.to_string(),
        });
    }

    let reclaimed = tasks
        .delete_errored_empty_task(task_id, viewer)
        .await
        .map_err(|error| {
            tracing::warn!(%error, task_id, "T3 thread create could not reclaim a failed bootstrap");
            CommandRefusal::Failed {
                message: "The thread could not be created.".to_string(),
            }
        })?;
    if !reclaimed {
        return Err(CommandRefusal::Rejected {
            message: "That thread already exists.".to_string(),
        });
    }

    tracing::info!(task_id, "T3 thread create reclaimed a failed bootstrap's id");
    Ok(())
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
            moatless_types::agent::AgentType(catalog::CLAUDE_CODE.id.to_string())
        );
        assert_eq!(
            default_agent_for("codex"),
            moatless_types::agent::AgentType(catalog::CODEX.id.to_string())
        );
    }

    /// An instance this server never published cannot arrive from a client that
    /// read our provider list, so it is a bug rather than a choice. Claude is
    /// the safe floor: it is the deployment's default harness.
    #[test]
    fn an_unknown_instance_falls_back_to_claude() {
        assert_eq!(
            default_agent_for("somethingElse"),
            moatless_types::agent::AgentType(
                moatless_types::agent::AgentType::CLAUDE_CODE.to_string()
            )
        );
    }

    /// The attach-PR dialog reads two `AdapterError` arms by name; the rest fall
    /// through a wildcard. Pin those two so a later rename or descent of the
    /// adapter error surface cannot silently reclassify a recognised failure as
    /// this deployment's own bug.
    #[test]
    fn the_two_recognised_adapter_errors_stay_actionable() {
        use moatless_adapter_model::error::{AdapterError, SharedAdapterError};

        let missing_credential = pull_request_error(AdapterError::upstream_credential(
            "resolve the GitHub token for user u-1",
            crate::auth::error::AuthError::Forbidden,
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

    mod thread_ids {
        use super::*;
        use crate::task::dao::MockTaskDao;
        use crate::task::models::Task;
        use moatless_types::access_control::Scope;

        const DRAFT: &str = "thread-minted-by-the-client";

        fn viewer() -> ScopeContext {
            ScopeContext {
                user_id: "u-1".to_string(),
                is_admin: false,
            }
        }

        fn thread_of(owner: &str, status: TaskStatus) -> Task {
            Task {
                task_id: DRAFT.to_string(),
                short_id: None,
                created_at: chrono::Utc::now(),
                last_message_at: chrono::Utc::now(),
                agent_type: moatless_types::agent::AgentType(
                    moatless_types::agent::AgentType::CLAUDE_CODE.to_string(),
                ),
                repository_id: "repo-1".to_string(),
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
                visibility: moatless_types::task::Visibility::Private,
                scope: Scope::User,
                owner_user_id: Some(owner.to_string()),
                updated_at: None,
                append_system_prompt: None,
                headless: false,
                ttl_minutes: 0,
                inactive_ttl_days: 0,
                loop_id: None,
                workspace_id: None,
                sandbox_id: None,
                checkout_mode: None,
                skills: Vec::new(),
            }
        }

        fn holding(existing: Option<Task>) -> MockTaskDao {
            let mut tasks = MockTaskDao::new();
            tasks
                .expect_find_by_id()
                .returning(move |_, _| Ok(existing.clone()));
            tasks
        }

        /// The ordinary create: nothing holds the id, so nothing is deleted on
        /// the way to it.
        #[tokio::test]
        async fn an_unused_id_is_taken_without_a_delete() {
            let mut tasks = holding(None);
            tasks.expect_delete_errored_empty_task().never();

            free_thread_id(&tasks, &viewer(), DRAFT).await.unwrap();
        }

        /// The bug this exists for: the first send failed at preflight, which
        /// left the row as `Error`, and the client re-sends the same draft id.
        /// Refusing that retry leaves the draft dead for good, because nothing
        /// on the client rotates the id.
        #[tokio::test]
        async fn a_failed_bootstrap_can_be_retried_under_the_same_id() {
            let mut tasks = holding(Some(thread_of("u-1", TaskStatus::Error)));
            tasks
                .expect_delete_errored_empty_task()
                .withf(|task_id, ctx| task_id == DRAFT && ctx.user_id == "u-1")
                .times(1)
                .returning(|_, _| Ok(true));

            free_thread_id(&tasks, &viewer(), DRAFT).await.unwrap();
        }

        /// A thread that exists and works is not something a create may take
        /// over. The DAO answers `false` for it — the row is not `Error`, or it
        /// has messages — and that is a refusal, not a second incarnation.
        #[tokio::test]
        async fn a_live_thread_still_refuses_a_second_create() {
            let mut tasks = holding(Some(thread_of("u-1", TaskStatus::Open)));
            tasks
                .expect_delete_errored_empty_task()
                .times(1)
                .returning(|_, _| Ok(false));

            let refusal = free_thread_id(&tasks, &viewer(), DRAFT).await.unwrap_err();
            assert!(matches!(
                refusal,
                CommandRefusal::Rejected { message } if message == "That thread already exists."
            ));
        }

        /// Somebody else's errored thread is readable when it is public, and it
        /// is still not this viewer's id to reuse. The refusal is the one that
        /// says nothing about what is there, and no delete is attempted.
        #[tokio::test]
        async fn another_users_errored_thread_is_not_reusable() {
            let mut tasks = holding(Some(thread_of("u-2", TaskStatus::Error)));
            tasks.expect_delete_errored_empty_task().never();

            let refusal = free_thread_id(&tasks, &viewer(), DRAFT).await.unwrap_err();
            assert!(matches!(
                refusal,
                CommandRefusal::NotWritable { task_id } if task_id == DRAFT
            ));
        }
    }
}
