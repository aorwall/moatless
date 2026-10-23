/// Route a parsed inbound PR event to its bound tasks, creating the review task
/// when the PR was just opened.
pub const PR_MESSAGE: &str = "github.pr_message";
/// Auto-close the tasks a closed/merged PR's trigger created.
pub const CLOSE: &str = "github.close";
/// Tell the tasks that outlive a merged PR that it landed.
pub const PR_MERGED: &str = "github.pr_merged";
/// Announce a base-branch CI run that completed after a merge.
pub const POST_MERGE: &str = "github.post_merge";
/// Re-check a PR's mergeable state and announce a new conflict.
pub const MERGE_CONFLICT: &str = "github.merge_conflict";
/// Read a failed check run's annotations and announce it to the PR's tasks.
pub const CHECK_RUN: &str = "github.check_run";
/// Open the Task a check-run trigger asks for, or add skills to it.
pub const CHECK_RUN_TRIGGER: &str = "github.check_run_trigger";
