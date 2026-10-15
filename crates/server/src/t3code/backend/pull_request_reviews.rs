//! [`PullRequestReviewSource`] for [`AppStateBoundPullRequests`]: a linked pull
//! request read from GitHub, and the Task that writes its guided review.

use super::commands::{AppStateCommands, create_refusal};
use super::pull_requests::{AppStateBoundPullRequests, pull_request_error};
use async_trait::async_trait;
use moatless_adapter_github_pr::adapter::GithubPrAdapter;
use moatless_adapter_github_pr::pull_request_search::{self, ListedPullRequest};
use moatless_adapter_github_pr::{
    pull_request_lookup,
    pull_request_search::{Involvement, PullRequestSearch},
};
use moatless_t3code::ports::{
    CommandRefusal, PullRequestError, PullRequestListItem, PullRequestListPage,
    PullRequestListQuery, PullRequestOverview, PullRequestPatchPage, PullRequestReviewSource,
};
use moatless_tasks::task::service as task_service;
use moatless_types::access_control::ScopeContext;
use moatless_types::agent::AgentProfile;
use moatless_workspace_types::repository::RepositoryProvider;
use std::sync::Arc;

#[async_trait]
impl PullRequestReviewSource for AppStateBoundPullRequests {
    async fn project_repository(
        &self,
        viewer: &ScopeContext,
        workspace_id: &str,
    ) -> Result<Option<String>, PullRequestError> {
        let store = |e: sqlx::Error| PullRequestError::Failed(e.to_string());
        let Some(repository_id) = self
            .0
            .workspaces
            .readable_primary_repository(viewer, workspace_id)
            .await
            .map_err(store)?
        else {
            return Ok(None);
        };
        let repository = self
            .0
            .repositories
            .find_by_id(&repository_id, viewer)
            .await
            .map_err(store)?;
        Ok(repository
            .and_then(|repository| repository.remote?.host_repo)
            .filter(|repo| repo.provider() == RepositoryProvider::Github)
            .map(|repo| format!("{}/{}", repo.owner(), repo.name())))
    }

    async fn list(
        &self,
        viewer: &ScopeContext,
        workspace_id: &str,
        repository: &str,
        query: &PullRequestListQuery,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<PullRequestListPage, PullRequestError> {
        let (owner, repo) = owner_repo(repository)?;
        let adapter = self.github_pr()?;
        let search = PullRequestSearch {
            state: query.state.clone(),
            involvement: match (query.involvement.as_deref(), query.viewer_login.clone()) {
                (Some("authored"), Some(login)) => Some(Involvement::Authored(login)),
                (Some("reviewing"), Some(login)) => Some(Involvement::ReviewRequested(login)),
                _ => None,
            },
            text: query.text.clone(),
            draft: query.draft.clone(),
            review: query.review.clone(),
            checks: query.checks.clone(),
            labels: query.labels.clone(),
            excluded_labels: query.excluded_labels.clone(),
            author: query.author.clone(),
        };
        let mut page = PullRequestListPage {
            next_cursor: cursor.map(str::to_string),
            ..Default::default()
        };
        // A first read has no cursor; later ones stop when the host runs out.
        let mut first = true;
        while (page.items.len() as u32) < limit && (first || page.next_cursor.is_some()) {
            first = false;
            let want = (limit - page.items.len() as u32).min(pull_request_search::MAX_PAGE_SIZE);
            let found = pull_request_search::search_pull_requests(
                adapter.as_ref(),
                &viewer.user_id,
                Some(workspace_id),
                owner,
                repo,
                &search,
                want,
                page.next_cursor.as_deref(),
            )
            .await
            .map_err(pull_request_error)?;
            page.items.extend(found.items.into_iter().map(list_item));
            page.next_cursor = found.next_cursor;
        }
        Ok(page)
    }

    async fn overview(
        &self,
        viewer: &ScopeContext,
        workspace_id: Option<&str>,
        repository: &str,
        number: u64,
    ) -> Result<PullRequestOverview, PullRequestError> {
        let (owner, repo) = owner_repo(repository)?;
        let pr = pull_request_lookup::pull_request_overview(
            self.github_pr()?.as_ref(),
            &viewer.user_id,
            workspace_id,
            owner,
            repo,
            number,
        )
        .await
        .map_err(pull_request_error)?;
        Ok(PullRequestOverview {
            number: pr.number,
            title: pr.title,
            body: pr.body,
            url: pr.url,
            state: pr.state,
            is_draft: pr.is_draft,
            author_login: pr.author_login,
            author_avatar_url: pr.author_avatar_url,
            additions: pr.additions,
            deletions: pr.deletions,
            changed_files: pr.changed_files,
            head_ref: pr.head_ref,
            head_sha: pr.head_sha,
            head_repository: pr.head_repository,
            base_ref: pr.base_ref,
            base_sha: pr.base_sha,
            created_at: pr.created_at,
            updated_at: pr.updated_at,
            merged_at: pr.merged_at,
            closed_at: pr.closed_at,
            mergeable: pr.mergeable,
        })
    }

    async fn patch_page(
        &self,
        viewer: &ScopeContext,
        workspace_id: Option<&str>,
        repository: &str,
        number: u64,
        page: u32,
    ) -> Result<PullRequestPatchPage, PullRequestError> {
        let (owner, repo) = owner_repo(repository)?;
        let page = pull_request_lookup::pull_request_patch_page(
            self.github_pr()?.as_ref(),
            &viewer.user_id,
            workspace_id,
            owner,
            repo,
            number,
            page,
        )
        .await
        .map_err(pull_request_error)?;
        Ok(PullRequestPatchPage {
            patch: page.patch,
            next_page: page.next_page,
            omitted_files: page.omitted_files,
        })
    }

    async fn create_review_guide_task(
        &self,
        viewer: &ScopeContext,
        workspace_id: &str,
        name: &str,
        message: String,
    ) -> Result<String, PullRequestError> {
        let request = moatless_api_schemas::task::CreateTaskRequest {
            repository_id: None,
            workspace_id: Some(workspace_id.to_string()),
            sandbox_id: None,
            checkout_mode: None,
            agent_profile: Some(AgentProfile::ClaudeCode.to_string()),
            name: Some(name.to_string()),
            description: None,
            // The agent fetches the pull request's head itself, which also
            // reaches a head on a fork.
            branch: None,
            message: Some(message),
            model: None,
            agent_mode: None,
            effort: None,
            contexts: vec![],
            file_ids: vec![],
            parent_task_id: None,
            run_as_user_id: None,
            headless: Some(false),
            skills: vec![],
        };
        let mut intent = task_service::CreateTaskIntent::new(request);
        // The caller attaches the Task to its run and then starts the sandbox.
        intent.provision = false;
        let commands = AppStateCommands(self.0.clone());
        let new_task =
            task_service::normalize(commands.create_ports(), viewer, &viewer.user_id, intent)
                .await
                .map_err(|error| refusal_error(create_refusal(workspace_id, error)))?;
        let outcome = task_service::create_task(
            &self.0.task_creation,
            self.0.message_send.clone(),
            new_task,
            |_| async { serde_json::Value::Null },
        )
        .await
        .map_err(|error| refusal_error(create_refusal(workspace_id, error)))?;
        Ok(outcome.task.task_id)
    }
}

impl AppStateBoundPullRequests {
    fn github_pr(&self) -> Result<Arc<GithubPrAdapter>, PullRequestError> {
        self.0
            .adapter_installations
            .as_ref()
            .map(|installations| installations.github_pr())
            .ok_or_else(|| {
                PullRequestError::Unavailable(
                    "This deployment has no GitHub integration installed.".to_string(),
                )
            })
    }
}

fn list_item(pr: ListedPullRequest) -> PullRequestListItem {
    PullRequestListItem {
        number: pr.number,
        title: pr.title,
        url: pr.url,
        author_login: pr.author_login,
        author_avatar_url: pr.author_avatar_url,
        author_is_bot: pr.author_is_bot,
        head_ref: pr.head_ref,
        base_ref: pr.base_ref,
        state: pr.state,
        is_draft: pr.is_draft,
        mergeability: pr.mergeability,
        additions: pr.additions,
        deletions: pr.deletions,
        created_at: pr.created_at,
        updated_at: pr.updated_at,
        labels: pr.labels,
        review_decision: pr.review_decision,
        checks_state: pr.checks_state,
        requested_reviewers: pr.requested_reviewers,
    }
}

fn owner_repo(repository: &str) -> Result<(&str, &str), PullRequestError> {
    repository
        .split_once('/')
        .filter(|(owner, repo)| !owner.is_empty() && !repo.is_empty() && !repo.contains('/'))
        .ok_or(PullRequestError::NotFound)
}

fn refusal_error(refusal: CommandRefusal) -> PullRequestError {
    match refusal {
        CommandRefusal::Rejected { message } => PullRequestError::Unavailable(message),
        CommandRefusal::NotWritable { .. } => {
            PullRequestError::Unavailable("You cannot start a task in this workspace.".to_string())
        }
        other => PullRequestError::Failed(format!("{other:?}")),
    }
}
