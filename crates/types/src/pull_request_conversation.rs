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
    /// `None` where the host did not say.
    pub can_edit: Option<bool>,
    pub reactions: Vec<PullRequestReaction>,
}

/// One kind of reaction on a remark or the pull request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestReaction {
    /// T3's name for it: `thumbs-up`, `heart`, ...
    pub content: String,
    pub count: u64,
    /// Logins of the first few who reacted.
    pub actors: Vec<String>,
    pub viewer_has_reacted: bool,
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
    pub can_edit: Option<bool>,
    pub reactions: Vec<PullRequestReaction>,
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
    /// Whether the viewer may resolve or unresolve it; `None` where unsaid.
    pub can_resolve: Option<bool>,
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
    /// On the pull request itself.
    pub reactions: Vec<PullRequestReaction>,
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
    /// `None` where the read did not say what the viewer may do.
    pub viewer: Option<PullRequestViewerRights>,
}

/// What the viewer may do to a pull request, and what its repository allows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PullRequestViewerRights {
    /// Write access or above to the base repository.
    pub can_write: bool,
    /// Triage access or above: labels.
    pub can_triage: bool,
    /// Write access to the head repository, whose branch still exists.
    pub can_delete_branch: bool,
    /// May edit, close, reopen or change the draft state.
    pub can_update: bool,
    pub did_author: bool,
    pub can_update_branch: bool,
    /// A merge would land now rather than wait on checks or reviews.
    pub can_merge_now: bool,
    pub merge_commit_allowed: bool,
    pub squash_merge_allowed: bool,
    pub rebase_merge_allowed: bool,
    pub auto_merge_allowed: bool,
    /// T3's name for the armed auto-merge's method; `None` when none is armed.
    pub auto_merge_method: Option<String>,
}

/// A changed file's line counts, for the change breakdown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestFileStat {
    pub path: String,
    pub additions: u64,
    pub deletions: u64,
}

/// A label of the repository's, and whether the pull request carries it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestLabelCandidate {
    pub name: String,
    pub color: Option<String>,
    pub description: Option<String>,
    pub is_applied: bool,
}

/// Someone who could be asked to review, and whether they are.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestReviewerCandidate {
    pub actor: PullRequestActor,
    pub is_requested: bool,
}

/// A page of candidates, `truncated` when the host holds more.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Candidates<T> {
    pub candidates: Vec<T>,
    pub truncated: bool,
}

/// How a merge or auto-merge combines the change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeMethod {
    Merge,
    Squash,
    Rebase,
}

/// A button on the pull request page, made as the viewer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PullRequestAction {
    Merge(MergeMethod),
    EnableAutoMerge(MergeMethod),
    DisableAutoMerge,
    Ready,
    Draft,
    Close,
    Reopen,
    DeleteBranch,
    /// `true` rebases the head onto the base; `false` merges the base in.
    UpdateBranch {
        rebase: bool,
    },
}

/// Where an inline review comment sits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewPosition {
    pub line: u64,
    pub side: DiffSide,
}

/// One inline comment of a submitted review.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewCommentDraft {
    pub path: String,
    pub position: ReviewPosition,
    /// The first line of a range ending at `position`.
    pub start: Option<ReviewPosition>,
    pub body: String,
}

/// The verdict a review gives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewVerdict {
    Comment,
    Approve,
    RequestChanges,
}

/// A write to a pull request or the remarks on it. Every id is the host's
/// node id, as the activity read hands them out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PullRequestWrite {
    Comment {
        body: String,
    },
    ReplyToThread {
        thread_id: String,
        body: String,
    },
    SetThreadResolution {
        thread_id: String,
        resolved: bool,
    },
    SubmitReview {
        verdict: ReviewVerdict,
        body: String,
        comments: Vec<ReviewCommentDraft>,
    },
    /// On `subject_id`, else on the pull request itself.
    SetReaction {
        subject_id: Option<String>,
        content: String,
        reacted: bool,
    },
    Update {
        title: Option<String>,
        body: Option<String>,
        maintainer_can_modify: Option<bool>,
    },
    UpdateComment {
        comment_id: String,
        /// A review comment rather than an issue comment.
        review_comment: bool,
        body: String,
    },
    SetLabels {
        labels: Vec<String>,
        applied: bool,
    },
    /// `users` by login, `teams` by slug.
    RequestReviewers {
        users: Vec<String>,
        teams: Vec<String>,
        requested: bool,
    },
    Action(PullRequestAction),
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
