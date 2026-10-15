//! A pull request's conversation, commits and status as its git host reports
//! them: what the T3 pull request page reads beyond the pull request itself.

/// Someone the host names on a pull request: an author, reviewer or committer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PullRequestActor {
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub is_bot: bool,
}

/// Which list of the host's a remark came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullRequestRemarkKind {
    IssueComment,
    ReviewComment,
    Review,
}

/// One entry of a pull request's conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestRemark {
    pub id: String,
    pub kind: PullRequestRemarkKind,
    pub author: Option<PullRequestActor>,
    pub body: String,
    /// RFC 3339, as every timestamp here.
    pub created_at: String,
    pub edited_at: Option<String>,
    pub url: Option<String>,
    /// The file a review comment is on.
    pub path: Option<String>,
    /// The host's review state (`APPROVED`, `CHANGES_REQUESTED`, ...) of a
    /// review.
    pub review_state: Option<String>,
}

/// One comment in a review thread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewThreadComment {
    pub id: String,
    pub author: Option<PullRequestActor>,
    pub body: String,
    pub created_at: String,
    pub edited_at: Option<String>,
    pub url: Option<String>,
}

/// Which version of a file a diff line belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffSide {
    /// Before the change.
    Left,
    /// After the change.
    Right,
}

/// A conversation anchored to a file of the diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewThread {
    pub id: String,
    pub path: String,
    /// `None` when the thread is on the file rather than on a line.
    pub line: Option<u64>,
    pub side: DiffSide,
    pub is_resolved: bool,
    pub is_outdated: bool,
    pub comments: Vec<ReviewThreadComment>,
    /// The host's count, which `comments` falls short of while
    /// `next_comments_cursor` is set.
    pub comment_count: u64,
    pub next_comments_cursor: Option<String>,
}

/// A page of one review thread's comments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReviewThreadCommentPage {
    pub comments: Vec<ReviewThreadComment>,
    pub next_cursor: Option<String>,
}

/// One commit of a pull request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestCommitEntry {
    pub oid: String,
    pub message_headline: String,
    pub committed_date: String,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub authors: Vec<PullRequestActor>,
}

/// A pull request's conversation and its newest commits.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PullRequestActivity {
    pub author: Option<PullRequestActor>,
    /// Everyone asked for a review or who has given one.
    pub reviewers: Vec<PullRequestActor>,
    /// Issue comments, reviews and review comments, oldest first.
    pub remarks: Vec<PullRequestRemark>,
    /// The host's own count of `remarks`, which is larger when
    /// `remarks_truncated`.
    pub remark_count: u64,
    pub remarks_truncated: bool,
    pub review_threads: Vec<ReviewThread>,
    pub review_threads_truncated: bool,
    /// Oldest first.
    pub commits: Vec<PullRequestCommitEntry>,
}

/// How one check on the head commit stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PullRequestCheckStatus {
    Pending,
    ActionRequired,
    Success,
    Failure,
    Skipped,
    Neutral,
    Cancelled,
}

/// A check run or commit status on the head commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestCheck {
    pub name: String,
    pub status: PullRequestCheckStatus,
    pub description: Option<String>,
    pub url: Option<String>,
    /// `None` where the host does not say whether the base branch requires it.
    pub required: Option<bool>,
}

/// A label, its colour hex without `#`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestLabel {
    pub name: String,
    pub color: Option<String>,
}

/// What a pull request carries besides its text: labels, requested reviewers
/// and the head commit's checks.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PullRequestStatus {
    pub labels: Vec<PullRequestLabel>,
    pub requested_reviewers: Vec<PullRequestActor>,
    pub checks: Vec<PullRequestCheck>,
}

/// How a file changed, as T3 names it when asking for both versions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileChangeType {
    Change,
    RenamePure,
    RenameChanged,
    New,
    Deleted,
}

/// A file before and after a change; a side the file does not exist on is
/// empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileContentsPair {
    pub old_contents: String,
    pub new_contents: String,
}
