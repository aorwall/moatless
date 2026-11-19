//! [`IssueTrackerSource`] over the Linear adapter, and the thread links
//! [`super::AppStateCommands`] makes through it.

use async_trait::async_trait;
use moatless_adapter_linear::issue_tracker::LinearIssueTracker;
use moatless_t3code::ports::{CommandRefusal, IssueTrackerSource, ThreadIssueRef};
use moatless_types::access_control::ScopeContext;
use moatless_types::issue_tracker::{
    CreatedIssue, IssueAssigneeCandidate, IssueCandidates, IssueCommentPage, IssueDetail,
    IssueDraft, IssueEdit, IssueListQuery, IssuePage, IssueTrackerError, IssueTrackerWorkspace,
};

/// Linear's issues, or `None` where Linear is not configured; built by
/// `FromRef<AppState>` in `crate::state`.
pub struct LinearIssues(pub Option<LinearIssueTracker>);

impl LinearIssues {
    fn tracker(&self) -> Result<&LinearIssueTracker, IssueTrackerError> {
        self.0.as_ref().ok_or(IssueTrackerError::Unauthenticated)
    }
}

#[async_trait]
impl IssueTrackerSource for LinearIssues {
    async fn workspaces(
        &self,
        viewer: &ScopeContext,
    ) -> Result<Vec<IssueTrackerWorkspace>, IssueTrackerError> {
        match &self.0 {
            Some(tracker) => tracker.workspaces(viewer).await,
            None => Ok(Vec::new()),
        }
    }

    async fn viewer(
        &self,
        viewer: &ScopeContext,
        scope: &str,
    ) -> Result<Option<IssueAssigneeCandidate>, IssueTrackerError> {
        self.tracker()?.viewer(viewer, scope).await
    }

    async fn list(
        &self,
        viewer: &ScopeContext,
        query: &IssueListQuery,
        viewer_id: Option<String>,
    ) -> Result<IssuePage, IssueTrackerError> {
        self.tracker()?
            .list(viewer, query, viewer_id.as_deref())
            .await
    }

    async fn detail(
        &self,
        viewer: &ScopeContext,
        scope: &str,
        key: &str,
    ) -> Result<IssueDetail, IssueTrackerError> {
        self.tracker()?.detail(viewer, scope, key).await
    }

    async fn comments(
        &self,
        viewer: &ScopeContext,
        scope: &str,
        key: &str,
        cursor: &str,
    ) -> Result<IssueCommentPage, IssueTrackerError> {
        self.tracker()?.comments(viewer, scope, key, cursor).await
    }

    async fn candidates(
        &self,
        viewer: &ScopeContext,
        scope: &str,
    ) -> Result<IssueCandidates, IssueTrackerError> {
        self.tracker()?.candidates(viewer, scope).await
    }

    async fn create(
        &self,
        viewer: &ScopeContext,
        login: &str,
        scope: &str,
        draft: IssueDraft,
    ) -> Result<CreatedIssue, IssueTrackerError> {
        self.tracker()?.create(viewer, login, scope, draft).await
    }

    async fn comment(
        &self,
        viewer: &ScopeContext,
        login: &str,
        scope: &str,
        key: &str,
        body: &str,
    ) -> Result<(), IssueTrackerError> {
        self.tracker()?
            .comment(viewer, login, scope, key, body)
            .await
    }

    async fn update(
        &self,
        viewer: &ScopeContext,
        scope: &str,
        key: &str,
        edit: IssueEdit,
    ) -> Result<(), IssueTrackerError> {
        self.tracker()?.update(viewer, scope, key, edit).await
    }

    async fn set_state(
        &self,
        viewer: &ScopeContext,
        scope: &str,
        key: &str,
        state: &str,
    ) -> Result<(), IssueTrackerError> {
        self.tracker()?.set_state(viewer, scope, key, state).await
    }

    async fn linked_tasks(
        &self,
        viewer: &ScopeContext,
        scope: &str,
        key: &str,
    ) -> Result<Vec<String>, IssueTrackerError> {
        self.tracker()?.linked_tasks(viewer, scope, key).await
    }
}

/// [`moatless_t3code::ports::ThreadCommandSink::link_issue`].
pub(super) async fn link_issue(
    tracker: Option<LinearIssueTracker>,
    viewer: &ScopeContext,
    reference: &ThreadIssueRef,
) -> Result<(), CommandRefusal> {
    let tracker = tracker.ok_or_else(not_configured)?;
    tracker
        .link(viewer, &reference.scope, &reference.key, &reference.task_id)
        .await
        .map_err(|error| link_refusal(reference, &error, "linked"))
}

/// [`moatless_t3code::ports::ThreadCommandSink::unlink_issue`].
pub(super) async fn unlink_issue(
    tracker: Option<LinearIssueTracker>,
    reference: &ThreadIssueRef,
) -> Result<(), CommandRefusal> {
    let tracker = tracker.ok_or_else(not_configured)?;
    tracker
        .unlink(&reference.task_id, &reference.key)
        .await
        .map_err(|error| link_refusal(reference, &error, "unlinked"))
}

fn not_configured() -> CommandRefusal {
    CommandRefusal::Failed {
        message: "Linear is not configured on this server.".to_string(),
    }
}

fn link_refusal(
    reference: &ThreadIssueRef,
    error: &IssueTrackerError,
    verb: &str,
) -> CommandRefusal {
    match error {
        IssueTrackerError::NotFound => CommandRefusal::Rejected {
            message: format!(
                "{} is not an issue in Linear team {}.",
                reference.key, reference.scope
            ),
        },
        IssueTrackerError::Refused(message) => CommandRefusal::Rejected {
            message: message.clone(),
        },
        IssueTrackerError::Unauthenticated | IssueTrackerError::Failed(_) => {
            tracing::warn!(?error, task_id = %reference.task_id, key = %reference.key, "T3 issue link failed");
            CommandRefusal::Failed {
                message: format!("That issue could not be {verb}."),
            }
        }
    }
}
