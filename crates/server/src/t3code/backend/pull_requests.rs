//! [`BoundPullRequestSource`], and the binding a thread reaches a pull request
//! through.

use crate::state::AppState;
use async_trait::async_trait;
use moatless_t3code::ports::{
    BoundPullRequest, BoundPullRequestSource, CommandRefusal, PullRequestError,
    ResolvedPullRequest, ThreadPullRequestRef,
};
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::TaskBinding;

pub struct AppStateBoundPullRequests(pub AppState, pub moatless_t3code::T3Reads);

#[async_trait]
impl BoundPullRequestSource for AppStateBoundPullRequests {
    async fn bound_pull_requests(&self, task_id: &str) -> Vec<BoundPullRequest> {
        let Ok(bindings) = self.0.task_bindings.for_task(task_id).await else {
            return Vec::new();
        };
        let mut bound = Vec::new();
        for binding in bindings.into_iter().filter(|binding| {
            binding.active
                && binding.adapter_kind == moatless_types::adapter::AdapterKind::Github
                && binding.subject_kind == "github_pr"
        }) {
            if let Some(pr) = bound_pull_request_from_binding(&binding) {
                bound.push(pr);
                continue;
            }
            // The status blob is still NULL — a freshly opened pull request no
            // surface has polled — so it resolves to nothing until fetched.
            // Refresh it once through the write-through, then rebuild from what
            // the fetch filled in. This walks every cold binding on the Task,
            // not only the one a caller asked after, so resolving one reference
            // warms the rest. A refresh that cannot be done (no token, API
            // error, no adapter) drops that pull request rather than the ones
            // beside it — and the listing, which tolerates a NULL blob, then
            // still links a reference this cannot resolve. That residual
            // disagreement is the state left when GitHub is unreachable.
            if let Some(pr) = self
                .refresh_incomplete_binding(task_id, &binding)
                .await
                .as_ref()
                .and_then(bound_pull_request_from_binding)
            {
                bound.push(pr);
            }
        }
        bound
    }

    async fn tasks_bound_to(&self, repository: &str, number: u64) -> Vec<String> {
        let subject_key = format!("{repository}#{number}");
        let Ok(bindings) = self
            .0
            .task_bindings
            .for_subject("github", "github_pr", &subject_key)
            .await
        else {
            return Vec::new();
        };
        // Task ids only, and no status refresh: the refresh spends the Task
        // creator's GitHub token, so it must not run before the caller has
        // authorized the viewer against these Tasks. The caller resolves the
        // pull request through `bound_pull_requests`, which does refresh, once
        // it has narrowed to a Task the viewer may read.
        bindings
            .into_iter()
            .filter(|binding| binding.active)
            .map(|binding| binding.task_id)
            .collect()
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
        moatless_adapter_github_pr::pull_request_lookup::resolve_reference_for_dialog(
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
        moatless_adapter_github_pr::pull_request_lookup::find_open_pull_request_for_head(
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
    /// return a copy carrying the freshly written-through fields. `None` when
    /// no live status could be obtained.
    async fn refresh_incomplete_binding(
        &self,
        task_id: &str,
        binding: &TaskBinding,
    ) -> Option<TaskBinding> {
        let created_by = self.task_creator(task_id).await?;
        let status = moatless_loops::bindings::status_service::get_or_fetch(
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
        self.1.task_creator(task_id).await.ok().flatten()
    }

    /// The `owner/repo` a repository's GitHub remote names, if the viewer can
    /// be shown the repository and it is a GitHub remote.
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

/// A GitHub API failure, mapped to what the attach-PR dialog is told.
///
/// Unmatched references become `NotFound`; a bad credential or host refusal
/// surfaces verbatim; anything else is a generic, logged failure.
pub(super) fn pull_request_error(
    error: moatless_adapter_model::error::AdapterError,
) -> PullRequestError {
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
    pr: moatless_adapter_github_pr::pull_request_lookup::DialogPullRequest,
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

/// Binds a newly created task to the open pull request on its branch, if any.
///
/// Best-effort: a missing PR, remote, or unreachable host leaves the thread
/// unbound rather than refusing its creation.
pub(super) async fn bind_thread_to_open_pr(
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
    let pr = match moatless_adapter_github_pr::pull_request_lookup::find_open_pull_request_for_head(
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
    let Some(connector) = state
        .adapter_installations
        .as_ref()
        .map(|installations| installations.github_connector())
    else {
        return;
    };
    match connector
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

/// Recover the pull request a GitHub binding points at, in the shape the status
/// surface reports. `None` when the persisted blob predates base and head, so
/// the toolbar shows no pull request rather than one with blank branches.
fn bound_pull_request_from_binding(binding: &TaskBinding) -> Option<BoundPullRequest> {
    let (repository, number) = binding.subject_key.rsplit_once('#')?;
    let number = number.parse().ok()?;
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
    let url = non_empty("url")
        .unwrap_or_else(|| format!("https://github.com/{repository}/pull/{number}"));
    let state_kind = binding
        .external_state_kind
        .clone()
        .unwrap_or_else(|| "open".to_string());
    Some(BoundPullRequest {
        number,
        repository: repository.to_string(),
        title,
        url,
        base_ref,
        head_ref,
        state_kind,
        // `last_event_at` stands in for a binding whose status has never been
        // refreshed: it moves whenever the adapter touches the binding, so it
        // never reports a fresher read than actually happened.
        status_read_at: binding
            .external_status_updated_at
            .unwrap_or(binding.last_event_at),
    })
}

/// Splits a `ThreadPullRequestRef` into `owner` and `repo`.
///
/// Only GitHub is bound; a reference on another host is refused by name.
pub(super) fn github_owner_repo(
    reference: &ThreadPullRequestRef,
) -> Result<(&str, &str), CommandRefusal> {
    if reference.host != "github.com" {
        return Err(CommandRefusal::Unsupported {
            command: "thread.pull-request.link".to_string(),
            reason: format!("`{}` pull requests are not supported.", reference.host),
        });
    }
    reference
        .repository
        .split_once('/')
        .filter(|(owner, repo)| !owner.is_empty() && !repo.is_empty())
        .ok_or_else(|| CommandRefusal::Rejected {
            message: format!(
                "`{}` is not an `owner/name` repository.",
                reference.repository
            ),
        })
}
