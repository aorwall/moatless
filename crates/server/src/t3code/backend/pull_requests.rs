//! [`BoundPullRequestSource`], and the binding a thread reaches a pull request
//! through.

use crate::state::AppState;
use async_trait::async_trait;
use moatless_ports::task::TaskReader;
use moatless_t3code::ports::{
    BoundPullRequest, BoundPullRequestSource, CommandRefusal, PullRequestError,
    ResolvedPullRequest, ThreadPullRequestRef,
};
use moatless_types::access_control::ScopeContext;
use moatless_types::adapter::TaskBinding;

pub struct AppStateBoundPullRequests(pub AppState);

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
            // A NULL status blob means nothing polled this PR yet, so it
            // refreshes once and rebuilds from the result — walking every
            // cold binding on the Task (not just the one asked after) so
            // resolving one warms the rest. A failed refresh drops only
            // that PR, leaving the listing's NULL-tolerant link unresolved.
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
    /// resolves.
    async fn task_creator(&self, task_id: &str) -> Option<String> {
        // reason: the port's caller has already checked the viewer's access to
        // this Task; the creator is read past it, under the system scope.
        #[allow(clippy::disallowed_methods)]
        let system = ScopeContext::system();
        self.0
            .task_reader
            .describe(task_id, &system)
            .await
            .ok()
            .flatten()?
            .created_by
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
        let remote = repository.remote.ok_or(PullRequestError::NotFound)?;
        remote
            .host_repo
            .filter(|repo| {
                repo.provider() == moatless_workspace_types::repository::RepositoryProvider::Github
            })
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
