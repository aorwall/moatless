//! An issue as a tracker reports it: what the T3 Issues page and issue panel
//! read, independent of which tracker answered.

pub use crate::pull_request_conversation::PullRequestActor as IssueActor;

/// A label on an issue, or one a team offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueLabel {
    pub name: String,
    /// Hex colour as the tracker reports it.
    pub color: Option<String>,
    pub description: Option<String>,
}

/// Where an issue stands, by the tracker's own name for the state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueState {
    pub name: String,
    pub closed: bool,
    /// How a closed state ended (`completed`, `canceled`); `None` while open.
    pub close_reason: Option<String>,
}

/// One row of a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueSummary {
    /// The tracker's container: a Linear team key.
    pub scope: String,
    /// The issue's native id in `scope`: a Linear identifier (`SP-12`).
    pub key: String,
    pub title: String,
    pub url: String,
    pub author: Option<IssueActor>,
    pub state: IssueState,
    /// RFC 3339, as every timestamp here.
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub assignees: Vec<IssueActor>,
    pub labels: Vec<IssueLabel>,
    pub comment_count: u64,
}

/// Which of the issues in a scope a listing asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueStateFilter {
    All,
    Open,
    Closed,
}

/// Which issues a listing narrows to by the viewer's part in them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueInvolvement {
    All,
    Assigned,
    Authored,
    Mentioned,
}

/// Newest first by the named date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueSort {
    Updated,
    Created,
}

/// One page of one scope's issues.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueListQuery {
    pub scope: String,
    pub state: IssueStateFilter,
    pub involvement: IssueInvolvement,
    pub query: Option<String>,
    /// Label names an issue must carry, every one of them.
    pub labels: Vec<String>,
    /// An assignee's login, as [`IssueActor::login`] names them.
    pub assignee: Option<String>,
    pub sort: IssueSort,
    pub limit: u32,
    pub after: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssuePage {
    pub items: Vec<IssueSummary>,
    /// Hand back as `after`; `None` on the last page.
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueComment {
    pub id: String,
    pub author: Option<IssueActor>,
    pub body: String,
    pub created_at: String,
    pub url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueCommentPage {
    pub comments: Vec<IssueComment>,
    pub next_cursor: Option<String>,
}

/// Something that happened to an issue that nobody wrote words for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueEvent {
    pub id: String,
    /// T3's vocabulary: `closed`, `reopened`, `labeled`, `assigned`, ...
    pub kind: &'static str,
    pub actor: Option<IssueActor>,
    pub created_at: String,
    pub detail: Option<String>,
}

/// A change request the tracker records against the issue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueLinkedPullRequest {
    pub repository: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    /// `open`, `closed` or `merged`.
    pub state: &'static str,
    pub is_draft: bool,
    /// Merging it closes the issue, rather than only citing it.
    pub closes_issue: bool,
}

/// An issue with its conversation, as the issue panel reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueDetail {
    pub summary: IssueSummary,
    pub body: String,
    /// Every state the issue can move to, in workflow order.
    pub states: Vec<IssueState>,
    pub comments: Vec<IssueComment>,
    pub comments_cursor: Option<String>,
    pub events: Vec<IssueEvent>,
    pub linked_pull_requests: Vec<IssueLinkedPullRequest>,
}

/// Someone an issue in a scope can be assigned to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueAssigneeCandidate {
    /// What a write names them by.
    pub id: String,
    pub actor: IssueActor,
}

/// The labels and people a scope offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueCandidates {
    pub labels: Vec<IssueLabel>,
    pub assignees: Vec<IssueAssigneeCandidate>,
    pub truncated: bool,
}

/// A new issue in a scope.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueDraft {
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
}

/// Fields an edit rewrites; `None` leaves one alone. `labels` and
/// `assignees` are the whole new set.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IssueEdit {
    pub title: Option<String>,
    pub body: Option<String>,
    pub labels: Option<Vec<String>>,
    pub assignees: Option<Vec<String>>,
}

/// An issue the tracker filed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreatedIssue {
    pub key: String,
    pub url: String,
}

/// Why a tracker call failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueTrackerError {
    /// No issue by that key, or the caller cannot see it.
    NotFound,
    /// The tracker has no connection the caller can use.
    Unauthenticated,
    /// The tracker refused the request; the message says why, for a person.
    Refused(String),
    /// This deployment's own failure, logged rather than shown.
    Failed(String),
}

/// One of a tracker's containers a connection reaches: a Linear team.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueTrackerTeam {
    pub key: String,
    pub name: String,
}

/// A tracker workspace this deployment is connected to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueTrackerWorkspace {
    /// As the tracker's issue URLs name it: Linear's `urlKey`.
    pub url_key: String,
    pub name: String,
    pub teams: Vec<IssueTrackerTeam>,
}
