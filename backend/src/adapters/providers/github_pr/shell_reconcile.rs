//! Reconcile GitHub state the agent changed out-of-band through the shell.
//!
//! The agent can mutate GitHub directly from its sandbox (`gh pr create`,
//! `gh pr comment`, `git push`). When it does, the adapter's view has to catch
//! up with what already happened:
//! - `gh pr create` → bind the task to the new PR, tag the assistant row so the
//!   chat renders its source chip, and append the `Moatless task: <url>` footer
//!   to the PR body.
//! - `gh pr comment` / `gh issue comment` → pre-claim the comment's external id
//!   so the inbound `issue_comment` webhook is recognised as our own.
//! - `gh api` POST to a review-comment, review or issue-comment endpoint →
//!   pre-claim the id the call returned. The reviewer loop's prompt tells
//!   agents to post inline comments exactly this way, so it is the ordinary
//!   path rather than an edge case.
//! - `git push` → claim the commit the push output names, so the
//!   `pull_request.synchronize` notification it produces (whose head commit
//!   starts with that abbreviated sha) is not delivered back to the task that
//!   pushed it.
//!
//! Recognition is never completed by guessing. A command shape this module
//! does not recognise, or one whose output does not name what it created,
//! records no claim — the event is delivered and the task reads back its own
//! comment, which costs one wasted turn. Claiming an id we did not observe
//! would instead suppress somebody else's event, which nobody notices. Every
//! branch that abandons a claim says so: see [`claim_abandoned`]. The
//! exceptions are the push shapes whose output shows nothing reached a GitHub
//! branch — a rejected push, an up-to-date push, a ref creation or deletion.
//! Those produce no notification at all, so there is no claim to abandon and
//! nothing to report.
//!
//! Reached by a direct call from the Message-domain post-persist hook, which
//! owns the message-shaped work (walking content blocks, resolving the tool
//! call, unwrapping the agent harness's shell wrapper) and hands the normalized
//! command line plus its output here. See ADR-0007: this is deliberately a
//! direct call rather than an `Adapter` interface method — it has exactly one
//! implementation.

use std::fmt::Write;

use serde_json::json;
use tracing::{debug, info, warn};

use super::api::GithubPrApi;
use crate::adapters::footer::FOOTER_MARKER;
use crate::adapters::metrics::record_shell_claim;
use crate::state::AppState;
use moatless_backend_core::scope::ScopeContext;

/// Adapter label on claim metrics and on the tagged assistant row.
const ADAPTER_LABEL: &str = "github_pr";

/// Posting shapes recognised through a `gh` subcommand or `git push`. The
/// `gh api` shapes name themselves — see [`GhApiCreated::shape`].
const SHAPE_PR_CREATE: &str = "pr_create";
const SHAPE_COMMENT: &str = "comment";
const SHAPE_PUSH: &str = "push";

/// Identity of a PR parsed from `gh pr create` output.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedPr {
    owner: String,
    repo: String,
    number: u64,
}

impl ParsedPr {
    fn pr_url(&self) -> String {
        format!(
            "https://github.com/{}/{}/pull/{}",
            self.owner, self.repo, self.number
        )
    }
}

/// Inspect one shell command the agent ran (and its output) for GitHub
/// mutations, and reconcile the adapter's view with what already happened.
///
/// `cmd` is the command line with the harness's shell wrapper already removed
/// and line continuations already joined; `result_text` is the command's
/// captured output. `tool_use_id` names the call that ran the command, and is
/// what the claim is recorded on: the turn the call was made from may have said
/// nothing and written no Message row, but the call itself always has one.
///
/// Errors are logged and swallowed — the event handler loop is
/// fire-and-forget.
pub(crate) async fn reconcile_shell_command(
    state: &AppState,
    task_id: &str,
    tool_use_id: &str,
    cmd: &str,
    result_text: &str,
) {
    if is_gh_pr_create(cmd) {
        handle_gh_pr_create(state, task_id, tool_use_id, result_text).await;
    } else if is_gh_pr_comment(cmd) || is_gh_issue_comment(cmd) {
        handle_gh_comment(state, task_id, tool_use_id, result_text).await;
    } else if let Some(created) = parse_gh_api_creation(cmd) {
        handle_gh_api_creation(state, task_id, tool_use_id, &created, result_text).await;
    } else if is_git_push(cmd) && !is_dry_run_push(cmd) {
        handle_git_push(state, task_id, tool_use_id, result_text).await;
    }
}

// =============================================================================
// Claim recording
// =============================================================================

/// Record this task's claim on the event its shell command just created: note
/// the id in the recently-sent cache and record it on the call that made it, so
/// the inbound webhook carrying the same id resolves back to this task.
///
/// `shape` is the fixed-vocabulary posting shape the command was recognised
/// as; it labels the claim metric and the log lines.
async fn record_claim(
    state: &AppState,
    task_id: &str,
    tool_use_id: &str,
    shape: &'static str,
    external_id: &str,
    source_name: &str,
) {
    // Note the in-memory claim before the (slower) row tag so an inbound
    // webhook cannot race past both.
    state.recently_sent.note(task_id, external_id);
    match state
        .messages
        .claim_activity_external_source(
            task_id,
            tool_use_id,
            external_id,
            ADAPTER_LABEL,
            Some(source_name),
        )
        .await
    {
        Ok(true) => {
            info!(
                task_id,
                tool_use_id,
                shape,
                external_id,
                "Claimed the event a shell GitHub mutation created"
            );
            record_shell_claim(ADAPTER_LABEL, shape, "claimed", "");
        }
        Ok(false) => {
            debug!(
                task_id,
                tool_use_id,
                shape,
                external_id,
                "The call already carried an external_event_id — leaving untouched"
            );
            record_shell_claim(ADAPTER_LABEL, shape, "claimed", "row_already_tagged");
        }
        Err(e) => {
            // The cache entry landed, but it is in-memory and short-lived; the
            // durable half of the claim is gone.
            warn!(task_id, tool_use_id, shape, external_id, error = %e, "Failed to record the claim on the call");
            record_shell_claim(ADAPTER_LABEL, shape, "unclaimed", "tag_failed");
        }
    }
}

/// Report a claim this module could not record.
///
/// Nothing is suppressed as a result — the event will be delivered and read
/// back by the task that caused it. `detail` names what the claim was for so
/// the echo can be traced back to the command that produced it.
fn claim_abandoned(task_id: &str, shape: &'static str, reason: &'static str, detail: &str) {
    warn!(
        task_id,
        shape,
        reason,
        detail,
        "Gave up claiming the event a shell GitHub mutation created — it will come back as an echo"
    );
    record_shell_claim(ADAPTER_LABEL, shape, "unclaimed", reason);
}

// =============================================================================
// gh pr create
// =============================================================================

async fn handle_gh_pr_create(
    state: &AppState,
    task_id: &str,
    tool_use_id: &str,
    result_text: &str,
) {
    let Some(pr) = parse_pr_url(result_text) else {
        claim_abandoned(task_id, SHAPE_PR_CREATE, "no_pr_url_in_output", "");
        return;
    };

    let source_id = format!("{}/{}", pr.owner, pr.repo);
    let thread_properties = json!({
        "owner": pr.owner,
        "repo": pr.repo,
        "pr_number": pr.number,
    });

    match crate::loops::firing::listen::connect_github_binding(
        state,
        &source_id,
        task_id,
        thread_properties,
        None,
    )
    .await
    {
        Ok((rule, binding)) => {
            info!(
                task_id,
                pr_number = pr.number,
                rule_id = rule.as_ref().map_or("<none>", |r| r.id.as_str()),
                binding_id = %binding.id,
                "Auto-connected GitHub PR binding for PR created by agent"
            );
        }
        Err(e) => {
            warn!(task_id, pr_number = pr.number, error = %e, "Failed to auto-connect GitHub PR binding");
        }
    }

    let external_id = super::external_id::pr_opened(pr.number);
    record_claim(
        state,
        task_id,
        tool_use_id,
        SHAPE_PR_CREATE,
        &external_id,
        &source_id,
    )
    .await;

    try_append_task_footer(state, task_id, &pr).await;
}

async fn try_append_task_footer(state: &AppState, task_id: &str, pr: &ParsedPr) {
    let frontend_url = state.config.auth.frontend_url.trim_end_matches('/');
    if frontend_url.is_empty() {
        debug!(task_id, "frontend_url empty — skipping PR body footer");
        return;
    }
    let task_url = format!("{frontend_url}/tasks/{task_id}");

    // reason: hook runs server-side after agent write; no user request context available.
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = ScopeContext::system();
    let task = match state.tasks.find_by_id(task_id, &sys_ctx).await {
        Ok(Some(t)) => t,
        Ok(None) => {
            warn!(task_id, "Task not found when appending PR footer");
            return;
        }
        Err(e) => {
            warn!(task_id, error = %e, "Failed to load task for PR footer");
            return;
        }
    };

    let token = match resolve_task_repository_github_token(state, &task, &pr.owner, &pr.repo).await
    {
        Ok(Some(t)) => t,
        Ok(None) => {
            warn!(
                task_id,
                "No repository-scoped GitHub credential — skipping PR footer"
            );
            return;
        }
        Err(e) => {
            warn!(task_id, error = %e, "Failed to resolve GitHub token for PR footer");
            return;
        }
    };

    let api = GithubPrApi::new(&token);
    let pr_info = match api.get_pr(&pr.owner, &pr.repo, pr.number).await {
        Ok(p) => p,
        Err(e) => {
            warn!(task_id, pr_url = %pr.pr_url(), error = %e, "Failed to fetch PR for footer");
            return;
        }
    };

    let existing = pr_info.body.as_deref().unwrap_or("");
    if existing.contains(FOOTER_MARKER) {
        return;
    }
    let new_body = compose_footer(existing, &task_url);

    if let Err(e) = api
        .update_pr_body(&pr.owner, &pr.repo, pr.number, &new_body)
        .await
    {
        warn!(task_id, pr_url = %pr.pr_url(), error = %e, "Failed to update PR body with footer");
        return;
    }
    info!(
        task_id,
        pr_number = pr.number,
        "Appended task URL footer to PR body"
    );
}

/// Append the `Moatless task: <url>` footer to a PR body. Also used by the
/// typed `POST /api/v1/github/.../pulls` endpoint to bake the footer into the
/// body at creation time.
pub(crate) fn compose_footer(existing: &str, task_url: &str) -> String {
    let trimmed = existing.trim_end_matches('\n').to_string();
    if trimmed.is_empty() {
        format!("{FOOTER_MARKER} {task_url}\n")
    } else {
        let mut out = trimmed;
        let _ = write!(out, "\n\n---\n{FOOTER_MARKER} {task_url}\n");
        out
    }
}

// =============================================================================
// gh pr comment / gh issue comment
// =============================================================================

async fn handle_gh_comment(state: &AppState, task_id: &str, tool_use_id: &str, result_text: &str) {
    let Some(parsed) = parse_comment_id(result_text) else {
        claim_abandoned(task_id, SHAPE_COMMENT, "no_comment_url_in_output", "");
        return;
    };

    // Wrap in the issue-comment namespace so the tagged row matches what the
    // GitHub `issue_comment` webhook will produce for the same comment.
    let external_id = super::external_id::issue_comment(parsed.id);
    let source_name = format!("{}/{}", parsed.owner, parsed.repo);
    record_claim(
        state,
        task_id,
        tool_use_id,
        SHAPE_COMMENT,
        &external_id,
        &source_name,
    )
    .await;
}

// =============================================================================
// gh api — raw provider API calls
// =============================================================================

/// What a raw `gh api` POST created, and where.
///
/// These three endpoints are the posting shapes that carry no footer and go
/// through neither the backend proxy nor a `gh` subcommand, so nothing else in
/// the system can recognise them as the task's own work.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GhApiCreation {
    owner: String,
    repo: String,
    kind: GhApiCreated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GhApiCreated {
    /// `POST repos/{owner}/{repo}/pulls/{n}/comments` — a new inline review
    /// comment, the shape the reviewer loop's prompt asks agents to use.
    ReviewComment,
    /// `POST repos/{owner}/{repo}/pulls/{n}/reviews` — a review submission.
    Review,
    /// `POST repos/{owner}/{repo}/issues/{n}/comments` — a conversation
    /// comment on a PR or an issue.
    IssueComment,
}

impl GhApiCreated {
    /// Fixed-vocabulary label for the claim metric and log lines.
    fn shape(self) -> &'static str {
        match self {
            Self::ReviewComment => "api_review_comment",
            Self::Review => "api_review",
            Self::IssueComment => "api_issue_comment",
        }
    }

    /// The fragment GitHub puts before the object's id in its `html_url`.
    fn html_anchor(self) -> &'static str {
        match self {
            Self::ReviewComment => "#discussion_r",
            Self::Review => "#pullrequestreview-",
            Self::IssueComment => "#issuecomment-",
        }
    }

    /// The external id the matching inbound webhook will carry.
    fn external_id(self, id: u64) -> String {
        match self {
            Self::ReviewComment => super::external_id::review_comment(id),
            Self::Review => super::external_id::review(id),
            Self::IssueComment => super::external_id::issue_comment(id),
        }
    }
}

async fn handle_gh_api_creation(
    state: &AppState,
    task_id: &str,
    tool_use_id: &str,
    created: &GhApiCreation,
    result_text: &str,
) {
    let source_name = format!("{}/{}", created.owner, created.repo);
    let shape = created.kind.shape();

    let Some(id) = parse_created_id(result_text, created.kind) else {
        claim_abandoned(task_id, shape, "no_id_in_response", &source_name);
        return;
    };

    let external_id = created.kind.external_id(id);
    record_claim(
        state,
        task_id,
        tool_use_id,
        shape,
        &external_id,
        &source_name,
    )
    .await;
}

// =============================================================================
// git push
// =============================================================================

/// Identity of a push target parsed from `git push` output: the GitHub
/// repository slug, the remote branch the push wrote to, and the commit that
/// branch now points at.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GitPushTarget {
    owner: String,
    repo: String,
    branch: String,
    /// The new-commit side of the status line's `<old>..<new>` range, as
    /// printed — abbreviated, and not yet checked for plausibility; that is
    /// [`crate::bindings::recently_sent::RecentlySent::note_commit`]'s call.
    ///
    /// `None` when the line carried no range at all: ref creation
    /// (`* [new branch]`) and deletion (`- [deleted]`) name no commit. A branch
    /// that did not exist on the remote has no pull request either, so no
    /// `synchronize` notification follows and nothing is owed a claim.
    commit: Option<String>,
}

/// Claim the commit the push wrote, so the `pull_request.synchronize` webhook
/// it produces is recognised as this task's own work.
///
/// The commit is read straight off the push output. Nothing is looked up and
/// nothing is fetched from GitHub: a reconstructed identifier (binding →
/// pull request number → API fetch → head sha) could name a commit other than
/// the one just pushed, and the round-trip widened the window in which the
/// webhook beat the claim.
///
/// A push to a branch with no pull request produces no notification, so the
/// claim simply expires unused — that is not a failure.
async fn handle_git_push(state: &AppState, task_id: &str, tool_use_id: &str, result_text: &str) {
    let Some(target) = parse_git_push(result_text) else {
        // Also the ordinary shape of a rejected push, "Everything up-to-date"
        // and a non-GitHub remote: nothing reached a GitHub branch, so no
        // notification follows and no claim is owed.
        debug!(
            task_id,
            "git push result named no GitHub repository and branch — nothing to claim"
        );
        return;
    };

    let repo_full = format!("{}/{}", target.owner, target.repo);
    let claim_detail = format!("{repo_full}#{}", target.branch);

    let Some(commit) = target.commit.as_deref() else {
        // Ref creation and deletion name no commit, and a branch that did not
        // exist on the remote has no pull request to notify.
        debug!(
            task_id,
            detail = %claim_detail,
            "git push created or deleted a ref — no commit to claim"
        );
        return;
    };

    // Unlike every other shape here, the push claim does not go through
    // [`record_claim`]: it is keyed on the commit rather than on an event id,
    // and matched by prefix (see [`RecentlySent::note_commit`]). That in-memory
    // claim is the whole of it — the row tag below only drives the chat's
    // source chip — so the claim outcome is recorded here, where it is decided.
    if !state.recently_sent.note_commit(task_id, commit) {
        // The store refuses a sha too short or malformed to name one commit,
        // because such a prefix would suppress commits the task never pushed.
        claim_abandoned(
            task_id,
            SHAPE_PUSH,
            "unclaimable_commit",
            &format!("{claim_detail}@{commit}"),
        );
        return;
    }
    info!(
        task_id,
        shape = SHAPE_PUSH,
        detail = %claim_detail,
        commit,
        "Claimed the commit a shell git push wrote"
    );
    record_shell_claim(ADAPTER_LABEL, SHAPE_PUSH, "claimed", "");

    // Record it on the call so the chat renders the source chip. The commit is
    // the only identifier this path has; the webhook's own
    // `github:pr:synchronize:<n>:<sha>` id cannot be formed without asking
    // GitHub for the pull request number, which is exactly the round-trip this
    // path no longer makes.
    let external_id = super::external_id::pushed_commit(commit);
    match state
        .messages
        .claim_activity_external_source(
            task_id,
            tool_use_id,
            &external_id,
            ADAPTER_LABEL,
            Some(&repo_full),
        )
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            debug!(
                task_id,
                tool_use_id,
                commit,
                "The call already carried an external_event_id — leaving untouched"
            );
        }
        // The claim itself already landed; only the source chip is lost.
        Err(e) => {
            warn!(task_id, tool_use_id, commit, error = %e, "Failed to tag assistant row with external_event_id for git push");
        }
    }
}

async fn resolve_task_repository_github_token(
    state: &AppState,
    task: &crate::task::models::Task,
    owner: &str,
    repo_name: &str,
) -> Result<Option<String>, crate::repository::error::RepositoryError> {
    // reason: hook runs in the agent's background flow; the task row was
    // already resolved for this flow and the repo is its configured one.
    #[allow(clippy::disallowed_methods)]
    let sys_ctx = ScopeContext::system();
    let repository = state
        .repositories
        .find_by_id(&task.repository_id, &sys_ctx)
        .await?
        .ok_or_else(|| {
            crate::repository::error::RepositoryError::NotFound(task.repository_id.clone())
        })?;
    let remote_url = repository.remote_url.as_deref().ok_or_else(|| {
        crate::repository::error::RepositoryError::BadRequest(
            "Task repository has no remote URL".to_string(),
        )
    })?;
    let persisted = crate::repository::access::parse_github_owner_repo(remote_url);
    if persisted
        .as_ref()
        .is_none_or(|(persisted_owner, persisted_repo)| {
            !persisted_owner.eq_ignore_ascii_case(owner)
                || !persisted_repo.eq_ignore_ascii_case(repo_name)
        })
    {
        return Err(crate::repository::error::RepositoryError::BadRequest(
            "GitHub target does not match the task repository".to_string(),
        ));
    }
    crate::repository::access::resolve_repository_github_token(state, &repository, remote_url).await
}

/// Extract the GitHub repository, remote branch and pushed commit from
/// `git push` stdout.
///
/// Looks for a `To <remote-url>` line to recover owner/repo (supports the
/// `https://github.com/<o>/<r>(.git)` and `git@github.com:<o>/<r>(.git)` URL
/// forms) and a status line containing `<local_ref> -> <remote_ref>` to
/// recover the branch and, from the summary field ahead of it, the commit.
/// Lines starting with `!` (e.g. `! [rejected]`) are skipped so a failed push
/// returns `None`. Returns `None` when the repository or the branch is missing
/// — covers `--dry-run` "Everything up-to-date" output, non-GitHub remotes,
/// and outright failures.
fn parse_git_push(text: &str) -> Option<GitPushTarget> {
    let mut owner_repo: Option<(String, String)> = None;
    let mut branch: Option<String> = None;
    let mut commit: Option<String> = None;

    for line in text.lines() {
        let trimmed = line.trim_start();

        if let Some(rest) = trimmed.strip_prefix("To ") {
            // `To` lines may carry a trailing `:` from `git push --porcelain`
            // wrappers; strip whitespace + the trailing colon before parsing.
            let url = rest.trim().trim_end_matches(':');
            if let Some(parsed) = parse_github_remote(url) {
                owner_repo = Some(parsed);
            }
            continue;
        }

        // Skip rejected / error status lines.
        if trimmed.starts_with('!') {
            continue;
        }

        if let Some(idx) = trimmed.find(" -> ") {
            let after = &trimmed[idx + 4..];
            if let Some(remote_ref) = after.split_whitespace().next() {
                if !remote_ref.is_empty() && !remote_ref.starts_with('(') {
                    branch = Some(remote_ref.to_string());
                    commit = parse_pushed_commit(&trimmed[..idx]);
                }
            }
        }
    }

    let (owner, repo) = owner_repo?;
    Some(GitPushTarget {
        owner,
        repo,
        branch: branch?,
        commit,
    })
}

/// Read the pushed commit out of the part of a status line that precedes the
/// `<local_ref> -> <remote_ref>` mapping.
///
/// The summary field is `<old>..<new>` for a fast-forward and `<old>...<new>`
/// for a forced update (`+ abc1234...def5678 fix/x -> fix/x`); the new commit
/// is the right-hand side. Ref creation and deletion print `[new branch]` /
/// `[deleted]` instead of a range, and carry no commit at all — hence `None`.
///
/// What comes back is whatever git printed. Whether it is long enough to name
/// one commit is [`crate::bindings::recently_sent::RecentlySent::note_commit`]'s
/// judgement, so that rule lives in one place.
fn parse_pushed_commit(before_mapping: &str) -> Option<String> {
    let range = before_mapping
        .split_whitespace()
        .find(|tok| tok.contains(".."))?;
    Some(range.rsplit('.').next().unwrap_or("").to_string())
}

/// Decompose a GitHub remote URL into `(owner, repo)`. Supports HTTPS, SSH
/// shorthand (`git@github.com:owner/repo`), and the explicit `ssh://` form.
fn parse_github_remote(url: &str) -> Option<(String, String)> {
    let path = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))
        .or_else(|| url.strip_prefix("git@github.com:"))?;
    let path = path.strip_suffix('/').unwrap_or(path);
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut parts = path.split('/');
    let owner = parts.next()?.to_string();
    let repo = parts.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

// =============================================================================
// Command parsing
// =============================================================================

/// Detect whether a `gh ...` command line invokes `pr create`.
///
/// Tolerates leading global flags (e.g. `gh -R owner/repo pr create ...`).
fn is_gh_pr_create(cmd: &str) -> bool {
    matches_gh_subcommand(cmd, &["pr", "create"])
}

fn is_gh_pr_comment(cmd: &str) -> bool {
    matches_gh_subcommand(cmd, &["pr", "comment"])
}

fn is_gh_issue_comment(cmd: &str) -> bool {
    matches_gh_subcommand(cmd, &["issue", "comment"])
}

/// Detect whether `cmd` invokes `git push` (in any form).
///
/// Tolerates leading env-var assignments (`GIT_DIR=… git push`), preceding
/// shell commands chained with `&&` / `||` / `;` / `|`
/// (`cd /tmp && git push`, `git add . && git push`), and `git`'s top-level
/// flags before the subcommand (`-C <path>`, `-c <name>=<value>`,
/// `--git-dir <path>`, `--work-tree <path>`).
///
/// Matches `git push --dry-run` too; [`is_dry_run_push`] is what rules it out
/// before the handler claims anything.
fn is_git_push(cmd: &str) -> bool {
    git_push_index(&cmd.split_whitespace().collect::<Vec<_>>()).is_some()
}

/// Index of the `push` subcommand token, when `tokens` is a `git push`
/// invocation. Shared by [`is_git_push`] and [`is_dry_run_push`] so both agree
/// on which token is the subcommand — a branch or remote *named* `push`
/// (`git push -n origin push`) must not be mistaken for it.
fn git_push_index(tokens: &[&str]) -> Option<usize> {
    // Skip past any preceding command (everything before the last shell
    // separator) and any env-var assignments, then check that the next token
    // is `git`.
    let after_separator = tokens
        .iter()
        .rposition(|t| SHELL_SEPARATORS.contains(t))
        .map_or(0, |i| i + 1);
    let mut idx = after_separator;
    while idx < tokens.len() && is_env_assignment(tokens[idx]) {
        idx += 1;
    }
    if tokens.get(idx) != Some(&"git") {
        return None;
    }

    let mut i = idx + 1;
    while i < tokens.len() {
        let tok = tokens[i];
        // Top-level git flags that take a separate value.
        if matches!(
            tok,
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace"
        ) {
            i += 2;
            continue;
        }
        if tok.starts_with('-') {
            i += 1;
            continue;
        }
        return (tok == "push").then_some(i);
    }
    None
}

/// Detect a dry-run push — `--dry-run`, `-n`, or an `n` inside a clustered
/// short-option group (`-qn`, `-nf`).
///
/// A dry run prints the same `<old>..<new>` range line a real push does, but
/// nothing reached the remote. Claiming the commit it names would suppress the
/// later, genuine push of that same commit — a silent drop of exactly the kind
/// this claim exists to prevent — so the *command* has to rule it out; the
/// output cannot.
///
/// Only the push's own flags count. The scan starts after the `push`
/// subcommand and stops at a shell separator (so neither `echo -n x && git
/// push` nor `git push && echo -n done` reads as a dry run) and at `--`, after
/// which tokens are refspecs rather than options.
fn is_dry_run_push(cmd: &str) -> bool {
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    let Some(push_idx) = git_push_index(&tokens) else {
        return false;
    };
    tokens[push_idx + 1..]
        .iter()
        .take_while(|t| !SHELL_SEPARATORS.contains(t) && **t != "--")
        .any(|t| is_dry_run_flag(t))
}

/// Whether one argument token is push's dry-run flag in any of its spellings.
///
/// `git` accepts clustered short options, so `-n` can arrive as `-qn` or
/// `-nf`. A cluster is scanned character by character and abandoned at `o`
/// (`--push-option`), whose value may be attached to the rest of the token —
/// `-omerge.notify=false` is a push option, not a dry run.
fn is_dry_run_flag(tok: &str) -> bool {
    if tok == "--dry-run" || tok.starts_with("--dry-run=") {
        return true;
    }
    let Some(cluster) = tok.strip_prefix('-') else {
        return false;
    };
    if cluster.starts_with('-') {
        return false;
    }
    for c in cluster.chars() {
        if c == 'n' {
            return true;
        }
        if c == 'o' {
            break;
        }
    }
    false
}

/// Shell tokens that separate one command from the next on the same line.
/// We treat tokens before the last such separator as "a preceding command we
/// don't care about" (e.g. `cd /tmp && gh pr create`).
const SHELL_SEPARATORS: &[&str] = &["&&", "||", ";", "|"];

/// Detect whether `cmd` is a `gh` invocation whose first two non-flag
/// positionals match `expected` (e.g. `["pr", "create"]`).
///
/// Tolerates leading env-var assignments (`GH_REPO=owner/repo gh pr create …`),
/// preceding shell commands chained with `&&` / `||` / `;` / `|`
/// (`cd /tmp && gh …`), a `moat` proxy token immediately before `gh`
/// (`moat gh pr create …`), and global flags like `-R owner/repo` /
/// `--repo owner/repo` before, between, and after the positionals.
///
/// Matching across shell separators is safe because the post-persist hooks
/// only fire when `parse_pr_url` / `parse_comment_id` finds a real URL in the
/// tool_result; if the chained `gh` never ran, there is no URL and we no-op.
fn matches_gh_subcommand(cmd: &str, expected: &[&str; 2]) -> bool {
    let tokens: Vec<&str> = cmd.split_whitespace().collect();
    let Some(args_start) = gh_args_start(&tokens) else {
        return false;
    };

    let mut iter = tokens.iter().skip(args_start).peekable();
    let mut matched = 0usize;
    while let Some(&tok) = iter.next() {
        if matches!(tok, "-R" | "--repo") {
            iter.next();
            continue;
        }
        if tok.starts_with('-') {
            continue;
        }
        if tok != expected[matched] {
            return false;
        }
        matched += 1;
        if matched == expected.len() {
            return true;
        }
    }
    false
}

/// Index of the first argument token after the `gh` executable, or `None` when
/// `tokens` is not a `gh` invocation.
///
/// Anything before the last shell separator is a preceding command we ignore
/// (`cd /tmp && gh …`); tokens between that separator and `gh` must be env-var
/// assignments (`GH_REPO=owner/repo gh …`) or the `moat` proxy token
/// (`moat gh …`).
fn gh_args_start<S: AsRef<str>>(tokens: &[S]) -> Option<usize> {
    let gh_idx = tokens.iter().position(|t| t.as_ref() == "gh")?;
    let prefix_start = tokens[..gh_idx]
        .iter()
        .rposition(|t| SHELL_SEPARATORS.contains(&t.as_ref()))
        .map_or(0, |i| i + 1);
    tokens[prefix_start..gh_idx]
        .iter()
        .all(|t| is_env_assignment(t.as_ref()) || t.as_ref() == "moat")
        .then_some(gh_idx + 1)
}

/// Detect a raw `gh api` call that creates a review comment, a review or an
/// issue comment, and name what it created.
///
/// A raw API call has no two-word subcommand for [`matches_gh_subcommand`] to
/// find — the target is the endpoint path, which can sit anywhere among the
/// flags. So the endpoint is recognised by its own shape rather than by
/// position, and the call must be a POST: `gh api` defaults to GET, and a read
/// of the same endpoint must never claim somebody else's comment.
fn parse_gh_api_creation(cmd: &str) -> Option<GhApiCreation> {
    let tokens = shell_tokens(cmd);
    let args_start = gh_args_start(&tokens)?;
    let args = tokens.get(args_start..)?;
    if args.first().map(String::as_str) != Some("api") {
        return None;
    }
    let args = &args[1..];
    if !gh_api_posts(args) {
        return None;
    }
    args.iter().find_map(|arg| parse_api_endpoint(arg))
}

/// Whether a `gh api` argument list issues a POST.
///
/// `gh api` sends GET unless the call carries a body (`-f` / `-F` /
/// `--input`), and an explicit `--method` / `-X` overrides either way.
fn gh_api_posts(args: &[String]) -> bool {
    let mut explicit: Option<String> = None;
    let mut has_body = false;

    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let arg = arg.as_str();
        if arg == "--method" || arg == "-X" {
            explicit = iter.next().map(|v| v.to_ascii_uppercase());
        } else if let Some(value) = arg
            .strip_prefix("--method=")
            .or_else(|| arg.strip_prefix("-X"))
            .filter(|v| !v.is_empty())
        {
            explicit = Some(value.to_ascii_uppercase());
        } else if matches!(arg, "-f" | "-F" | "--field" | "--raw-field" | "--input") {
            has_body = true;
            iter.next();
        } else if (arg.starts_with("-f") || arg.starts_with("-F"))
            || arg.starts_with("--field=")
            || arg.starts_with("--raw-field=")
            || arg.starts_with("--input=")
        {
            has_body = true;
        }
    }

    explicit.map_or(has_body, |method| method == "POST")
}

/// Decompose a `gh api` endpoint argument into the repository it targets and
/// the kind of event a POST to it creates.
///
/// Accepts the bare (`repos/o/r/…`), rooted (`/repos/o/r/…`) and absolute
/// (`https://api.github.com/repos/o/r/…`) spellings, and ignores a query
/// string. Any other endpoint — a read, an unrelated resource, or a deeper
/// path such as `…/comments/{id}/replies` — returns `None`.
fn parse_api_endpoint(arg: &str) -> Option<GhApiCreation> {
    let path = arg
        .strip_prefix("https://api.github.com/")
        .or_else(|| arg.strip_prefix("http://api.github.com/"))
        .unwrap_or(arg);
    let path = path.split(['?', '#']).next()?;
    let path = path.trim_matches('/');

    let mut parts = path.split('/');
    if parts.next()? != "repos" {
        return None;
    }
    let owner = parts.next()?;
    let repo = parts.next()?;
    let subject = parts.next()?;
    let number = parts.next()?;
    let leaf = parts.next()?;
    if parts.next().is_some() || owner.is_empty() || repo.is_empty() {
        return None;
    }
    if number.parse::<u64>().is_err() {
        return None;
    }

    let kind = match (subject, leaf) {
        ("pulls", "comments") => GhApiCreated::ReviewComment,
        ("pulls", "reviews") => GhApiCreated::Review,
        ("issues", "comments") => GhApiCreated::IssueComment,
        _ => return None,
    };

    Some(GhApiCreation {
        owner: owner.to_string(),
        repo: repo.to_string(),
        kind,
    })
}

/// Split a command line into tokens, honouring single quotes, double quotes
/// and backslash escapes, and dropping the quote characters.
///
/// The subcommand matcher gets away with `split_whitespace` because its
/// positionals are single words in fixed places. An endpoint path is found by
/// shape anywhere in the argument list, so quoting has to be respected: without
/// it, `-H "Accept: application/vnd.github+json"` and a comment body that
/// mentions a path would both split into loose tokens that could be mistaken
/// for the endpoint.
fn shell_tokens(cmd: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quote: Option<char> = None;
    let mut chars = cmd.chars();

    while let Some(c) = chars.next() {
        match quote {
            Some(q) => {
                if c == q {
                    quote = None;
                } else if c == '\\' && q == '"' {
                    current.extend(chars.next());
                } else {
                    current.push(c);
                }
            }
            None if c == '\'' || c == '"' => {
                quote = Some(c);
                started = true;
            }
            None if c == '\\' => {
                current.extend(chars.next());
                started = true;
            }
            None if c.is_whitespace() => {
                if started {
                    tokens.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            None => {
                current.push(c);
                started = true;
            }
        }
    }
    if started {
        tokens.push(current);
    }
    tokens
}

/// `KEY=VALUE` form, where KEY is an alphanumeric/underscore identifier.
fn is_env_assignment(tok: &str) -> bool {
    let Some(eq) = tok.find('=') else {
        return false;
    };
    let key = &tok[..eq];
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// =============================================================================
// Output parsing
// =============================================================================

/// Find the LAST GitHub PR URL in `text` and decompose into owner/repo/number.
fn parse_pr_url(text: &str) -> Option<ParsedPr> {
    let needle = "https://github.com/";
    let mut last: Option<ParsedPr> = None;
    let mut start = 0;
    while let Some(idx) = text[start..].find(needle) {
        let abs = start + idx;
        let rest = &text[abs..];
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
            .unwrap_or(rest.len());
        let url = &rest[..end];
        if let Some(pr) = decompose_pr_url(url) {
            last = Some(pr);
        }
        start = abs + needle.len();
    }
    last
}

fn decompose_pr_url(url: &str) -> Option<ParsedPr> {
    let path = url.strip_prefix("https://github.com/")?;
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    if parts.next() != Some("pull") {
        return None;
    }
    let number_token = parts.next()?;
    // Strip `#issuecomment-...` or trailing path segments from the number.
    let number_str = number_token
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or("");
    let number: u64 = number_str.parse().ok()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some(ParsedPr {
        owner: owner.to_string(),
        repo: repo.to_string(),
        number,
    })
}

/// Identity of an issue/PR comment URL parsed from `gh pr/issue comment`
/// output.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedComment {
    owner: String,
    repo: String,
    id: u64,
}

/// Extract owner/repo + the numeric id after `#issuecomment-` in
/// `gh pr/issue comment` output (e.g.
/// `https://github.com/foo/bar/pull/42#issuecomment-12345`).
///
/// Returns `None` when no `#issuecomment-` URL is present or the URL shape
/// doesn't have the expected `owner/repo/(pull|issues)/N#issuecomment-N`
/// segments.
fn parse_comment_id(text: &str) -> Option<ParsedComment> {
    let needle = "#issuecomment-";
    let idx = text.find(needle)?;

    // Walk backwards to the start of the URL.
    let prefix = "https://github.com/";
    let url_start = text[..idx].rfind(prefix)?;
    let url = &text[url_start..idx];

    // url is `https://github.com/<owner>/<repo>/<pull|issues>/<n>`.
    let path = url.strip_prefix(prefix)?;
    let mut parts = path.split('/');
    let owner = parts.next()?;
    let repo = parts.next()?;
    let kind = parts.next()?;
    if kind != "pull" && kind != "issues" {
        return None;
    }

    let rest = &text[idx + needle.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let id: u64 = rest[..end].parse().ok()?;

    if owner.is_empty() || repo.is_empty() {
        return None;
    }

    Some(ParsedComment {
        owner: owner.to_string(),
        repo: repo.to_string(),
        id,
    })
}

/// Read the id GitHub assigned to the object a raw `gh api` POST created.
///
/// Prefers the `id` field of the JSON body `gh api` prints. Falls back to the
/// anchor in that object's `html_url` — `#discussion_r…`, `#pullrequestreview-…`
/// or `#issuecomment-…` — which survives output the JSON parse cannot take,
/// such as a body preceded by other command output or truncated by the
/// harness. Both read the id back out of the response: an id reconstructed
/// from the request would be a guess, and a wrong claim silently suppresses
/// somebody else's event.
fn parse_created_id(text: &str, kind: GhApiCreated) -> Option<u64> {
    if let Ok(serde_json::Value::Object(body)) = serde_json::from_str(text.trim()) {
        if let Some(id) = body.get("id").and_then(serde_json::Value::as_u64) {
            return Some(id);
        }
    }
    last_id_after(text, kind.html_anchor())
}

/// The digits following the last occurrence of `anchor` in `text`.
fn last_id_after(text: &str, anchor: &str) -> Option<u64> {
    let idx = text.rfind(anchor)?;
    let rest = &text[idx + anchor.len()..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;
    // The Message domain owns unwrapping the agent harness's shell wrapper and
    // joining line continuations; these tests assert the GitHub classifiers
    // still match what it hands over.
    use crate::bindings::recently_sent::RecentlySent;
    use crate::message::hooks::{normalize_line_continuations, unwrap_shell_wrapper};

    #[test]
    fn parse_pr_url_basic() {
        let pr = parse_pr_url("https://github.com/foo/bar/pull/42\n").unwrap();
        assert_eq!(
            pr,
            ParsedPr {
                owner: "foo".into(),
                repo: "bar".into(),
                number: 42
            }
        );
    }

    #[test]
    fn parse_pr_url_picks_last_match() {
        let input = "https://github.com/a/b/pull/1\nhttps://github.com/c/d/pull/2";
        let pr = parse_pr_url(input).unwrap();
        assert_eq!(pr.owner, "c");
        assert_eq!(pr.number, 2);
    }

    #[test]
    fn parse_pr_url_none_for_browser_message() {
        assert!(parse_pr_url("Opening github.com in your browser.\n").is_none());
    }

    #[test]
    fn parse_pr_url_ignores_non_pr_urls() {
        assert!(parse_pr_url("https://github.com/foo/bar/issues/42\n").is_none());
        assert!(parse_pr_url("https://github.com/foo/bar\n").is_none());
        assert!(parse_pr_url("plain text with no urls").is_none());
    }

    #[test]
    fn is_gh_pr_create_simple() {
        assert!(is_gh_pr_create("gh pr create"));
        assert!(is_gh_pr_create("gh pr create --title foo --body bar"));
    }

    #[test]
    fn is_gh_pr_create_with_flags() {
        assert!(is_gh_pr_create("gh pr --draft create --title x"));
    }

    #[test]
    fn is_gh_pr_create_with_dash_r() {
        assert!(is_gh_pr_create("gh -R o/r pr create"));
        assert!(is_gh_pr_create("gh --repo o/r pr create"));
    }

    #[test]
    fn is_gh_pr_create_negative() {
        assert!(!is_gh_pr_create("gh pr view 42"));
        assert!(!is_gh_pr_create("gh issue create"));
        assert!(!is_gh_pr_create(""));
        assert!(!is_gh_pr_create("git pr create"));
        assert!(!is_gh_pr_create("cd /tmp ; echo done"));
    }

    #[test]
    fn is_gh_pr_create_with_chained_prefix() {
        // The original bug: `cd <path> && gh pr create ...` was rejected.
        assert!(is_gh_pr_create(
            "cd /tmp/soap-k8s && gh pr create --head fix/x --title \"y z\""
        ));
        // Combined shell prefix and env assignment between separator and `gh`.
        assert!(is_gh_pr_create("cd /tmp && GH_REPO=o/r gh pr create"));
        // Other separators behave the same. The earlier conservative test
        // (`git push && gh pr create` should NOT match) is intentionally
        // dropped: matching is harmless because the URL parser no-ops when
        // the chained `gh` did not run.
        assert!(is_gh_pr_create("git push && gh pr create"));
        assert!(is_gh_pr_create("git push || gh pr create"));
        assert!(is_gh_pr_create("cd /tmp ; gh pr create"));
    }

    #[test]
    fn is_gh_pr_create_with_line_continuation() {
        // The PR #787 incident shape: backslash-newline continuation between
        // the chained `cd` and the `gh` invocation.
        let raw = "cd /opt/moatless/soap-backend && \\\ngh pr create --head feat/x --title \"fix\"";
        let cmd = normalize_line_continuations(raw);
        assert!(is_gh_pr_create(cmd.as_ref()));
    }

    #[test]
    fn is_git_push_with_line_continuation() {
        let raw = "cd /opt/moatless/soap-backend && \\\ngit push -u origin feat/x";
        let cmd = normalize_line_continuations(raw);
        assert!(is_git_push(cmd.as_ref()));
    }

    #[test]
    fn normalize_line_continuations_passthrough_when_absent() {
        let raw = "gh pr create --title x";
        assert!(matches!(
            normalize_line_continuations(raw),
            Cow::Borrowed(_)
        ));
        // CRLF continuations are joined too.
        let crlf = "cd /tmp && \\\r\ngh pr create";
        assert!(is_gh_pr_create(normalize_line_continuations(crlf).as_ref()));
    }

    #[test]
    fn is_gh_mutation_with_moat_proxy_prefix() {
        assert!(is_gh_pr_create("moat gh pr create --title t"));
        assert!(is_gh_pr_create("cd /x && moat gh pr create --title t"));
        assert!(is_gh_pr_comment("moat gh pr comment 42 --body hi"));
        assert!(is_gh_issue_comment("moat gh issue comment 7 --body hi"));
        // `moat` plus env assignment combinations still match.
        assert!(is_gh_pr_create("GH_REPO=o/r moat gh pr create --title t"));
        // Unrelated leading words still do not match.
        assert!(!is_gh_pr_create("echo gh pr create"));
    }

    #[test]
    fn is_gh_pr_comment_with_chained_prefix() {
        assert!(is_gh_pr_comment("cd /tmp && gh pr comment 42 --body hi"));
        assert!(is_gh_issue_comment(
            "cd /tmp && gh issue comment 1 --body hi"
        ));
    }

    #[test]
    fn is_gh_pr_create_with_env_prefix() {
        assert!(is_gh_pr_create(
            "GH_REPO=soaplabs/foo gh pr create --title bar"
        ));
        assert!(is_gh_pr_create(
            "GH_REPO=soaplabs/foo   gh pr create --base main --head fix/foo --title \"x y\" --body-file /tmp/b.md 2>&1 | tee /tmp/out"
        ));
        assert!(is_gh_pr_create("FOO=1 BAR=2 gh pr create"));
    }

    #[test]
    fn is_gh_pr_comment_simple() {
        assert!(is_gh_pr_comment("gh pr comment 42 --body hi"));
        assert!(is_gh_pr_comment("gh -R o/r pr comment 42"));
        assert!(!is_gh_pr_comment("gh pr create"));
        assert!(!is_gh_pr_comment("gh issue comment 1"));
    }

    #[test]
    fn is_gh_issue_comment_simple() {
        assert!(is_gh_issue_comment("gh issue comment 1 --body hi"));
        assert!(!is_gh_issue_comment("gh pr comment 1"));
    }

    #[test]
    fn parse_comment_id_basic() {
        let parsed =
            parse_comment_id("https://github.com/foo/bar/pull/42#issuecomment-12345\n").unwrap();
        assert_eq!(
            parsed,
            ParsedComment {
                owner: "foo".into(),
                repo: "bar".into(),
                id: 12345,
            }
        );
    }

    #[test]
    fn parse_comment_id_for_issue_url() {
        let parsed =
            parse_comment_id("https://github.com/foo/bar/issues/7#issuecomment-99\n").unwrap();
        assert_eq!(parsed.owner, "foo");
        assert_eq!(parsed.repo, "bar");
        assert_eq!(parsed.id, 99);
    }

    #[test]
    fn parse_comment_id_none() {
        assert!(parse_comment_id("no comment url here").is_none());
        assert!(parse_comment_id("https://github.com/foo/bar/pull/42").is_none());
        // Wrong path segment.
        assert!(
            parse_comment_id("https://github.com/foo/bar/discussions/1#issuecomment-1").is_none(),
        );
    }

    #[test]
    fn footer_appends_to_existing_body() {
        let body = "## Summary\nFix bug.\n\n## Test plan\n- [x] Tests";
        let out = compose_footer(body, "https://moatless.dev/tasks/t_1");
        assert!(out.starts_with("## Summary"));
        assert!(out.contains("---\nMoatless task: https://moatless.dev/tasks/t_1\n"));
        assert!(out.contains("- [x] Tests"));
    }

    #[test]
    fn footer_handles_empty_body() {
        let out = compose_footer("", "https://moatless.dev/tasks/t_2");
        assert_eq!(out, "Moatless task: https://moatless.dev/tasks/t_2\n");
    }

    #[test]
    fn footer_trims_trailing_newlines_before_separator() {
        let out = compose_footer("body\n\n\n", "https://x/tasks/t");
        assert_eq!(out, "body\n\n---\nMoatless task: https://x/tasks/t\n");
    }

    #[test]
    fn is_gh_pr_create_codex_wrapped() {
        let raw = "/bin/bash -lc 'gh pr create --title foo --body bar'";
        assert!(is_gh_pr_create(unwrap_shell_wrapper(raw)));
    }

    #[test]
    fn is_gh_pr_comment_codex_wrapped() {
        let raw = "/bin/bash -lc 'gh pr comment 42 --body hi'";
        assert!(is_gh_pr_comment(unwrap_shell_wrapper(raw)));
    }

    #[test]
    fn is_gh_issue_comment_codex_wrapped() {
        let raw = "/bin/bash -lc 'gh issue comment 1 --body hi'";
        assert!(is_gh_issue_comment(unwrap_shell_wrapper(raw)));
    }

    #[test]
    fn is_gh_pr_create_codex_with_repo_flag() {
        let raw = "/bin/bash -lc 'gh -R o/r pr create'";
        assert!(is_gh_pr_create(unwrap_shell_wrapper(raw)));
    }

    #[test]
    fn is_gh_pr_create_codex_with_chained_cd() {
        let raw = "/bin/bash -lc 'cd /tmp/soap-k8s && gh pr create --head fix/x'";
        assert!(is_gh_pr_create(unwrap_shell_wrapper(raw)));
    }

    // ===== git push classifier =====

    #[test]
    fn is_git_push_simple() {
        assert!(is_git_push("git push"));
        assert!(is_git_push("git push origin HEAD"));
        assert!(is_git_push("git push -u origin fix/x --force-with-lease"));
        assert!(is_git_push("git push --dry-run"));
    }

    #[test]
    fn is_git_push_with_chained_prefix() {
        assert!(is_git_push("cd /tmp && git push"));
        assert!(is_git_push("git add . && git push"));
        assert!(is_git_push("cd /tmp ; git push origin HEAD"));
        assert!(is_git_push("FOO=1 git push"));
    }

    #[test]
    fn is_git_push_with_top_level_flag_value() {
        assert!(is_git_push("git -C /tmp/repo push"));
        assert!(is_git_push("git --git-dir /tmp/repo/.git push"));
    }

    #[test]
    fn is_git_push_negative() {
        assert!(!is_git_push("git status"));
        assert!(!is_git_push("git pull"));
        assert!(!is_git_push("gh pr create"));
        assert!(!is_git_push(""));
        // `push` not as a git subcommand.
        assert!(!is_git_push("npm push"));
    }

    #[test]
    fn is_git_push_codex_wrapped() {
        let raw = "/bin/bash -lc 'git push'";
        assert!(is_git_push(unwrap_shell_wrapper(raw)));
        let raw = "/bin/bash -lc 'cd /tmp/soap-config && git push --force-with-lease'";
        assert!(is_git_push(unwrap_shell_wrapper(raw)));
    }

    // ===== parse_git_push =====

    #[test]
    fn parse_git_push_https() {
        let out = "Enumerating objects: 5, done.\n\
                   Counting objects: 100% (5/5), done.\n\
                   To https://github.com/soaplabs/soap-config.git\n   \
                   2fa9788..7fc0612  fix/TM-1331-prevent-orphan-draft-branches -> fix/TM-1331-prevent-orphan-draft-branches\n";
        let target = parse_git_push(out).unwrap();
        assert_eq!(
            target,
            GitPushTarget {
                owner: "soaplabs".into(),
                repo: "soap-config".into(),
                branch: "fix/TM-1331-prevent-orphan-draft-branches".into(),
                commit: Some("7fc0612".into()),
            }
        );
    }

    #[test]
    fn parse_git_push_ssh() {
        let out = "To git@github.com:owner/repo.git\n   abc1234..def5678  fix/x -> fix/x\n";
        let target = parse_git_push(out).unwrap();
        assert_eq!(
            target,
            GitPushTarget {
                owner: "owner".into(),
                repo: "repo".into(),
                branch: "fix/x".into(),
                commit: Some("def5678".into()),
            }
        );
    }

    #[test]
    fn parse_git_push_force_update_uses_remote_ref() {
        // `+ ...` lines (force update) still carry the `local -> remote` form,
        // and a `...` range whose right-hand side is the commit now on the
        // branch.
        let out =
            "To https://github.com/o/r.git\n + abc1234...def5678 fix/x -> fix/x (forced update)\n";
        let target = parse_git_push(out).unwrap();
        assert_eq!(target.branch, "fix/x");
        assert_eq!(target.commit.as_deref(), Some("def5678"));
    }

    #[test]
    fn parse_git_push_reads_a_full_length_sha_too() {
        let long = "0123456789abcdef0123456789abcdef01234567";
        let out = format!("To https://github.com/o/r.git\n   abc1234..{long}  fix/x -> fix/x\n");
        assert_eq!(parse_git_push(&out).unwrap().commit.as_deref(), Some(long));
    }

    #[test]
    fn parse_git_push_new_branch_has_no_commit() {
        // A branch that did not exist on the remote has no pull request, so no
        // `synchronize` notification follows and nothing is owed a claim.
        let out = "To https://github.com/o/r.git\n * [new branch]      fix/x -> fix/x\n";
        let target = parse_git_push(out).unwrap();
        assert_eq!(target.branch, "fix/x");
        assert_eq!(target.commit, None);

        let out = "To https://github.com/o/r.git\n - [deleted]         fix/x\n";
        assert!(parse_git_push(out).is_none());
    }

    #[test]
    fn parse_git_push_hands_an_implausible_abbreviation_to_the_claim_guard() {
        // The parser reports what git printed; `note_commit` refuses a prefix
        // this short, and `handle_git_push` warns and counts rather than
        // skipping silently.
        let out = "To https://github.com/o/r.git\n   abc..def  fix/x -> fix/x\n";
        let commit = parse_git_push(out).unwrap().commit.unwrap();
        assert_eq!(commit, "def");
        assert!(!RecentlySent::new().note_commit("task-1", &commit));
    }

    #[test]
    fn parse_git_push_dry_run_returns_none() {
        // Output without a range line (e.g. the "Everything up-to-date" case).
        let out = "Everything up-to-date\n";
        assert!(parse_git_push(out).is_none());

        // No `To` line either.
        assert!(parse_git_push("").is_none());
    }

    #[test]
    fn dry_run_push_is_not_claimed() {
        // `--dry-run` prints the same range line a real push does, so the
        // command — not the output — is what rules it out.
        assert!(is_dry_run_push("git push --dry-run"));
        assert!(is_dry_run_push("cd /tmp && git push -n origin HEAD"));
        assert!(is_dry_run_push("git -c push.default=simple push --dry-run"));
        assert!(!is_dry_run_push("git push -u origin fix/x"));
    }

    #[test]
    fn dry_run_push_detects_clustered_short_flags() {
        // git accepts clustered short options, so `-n` arrives in company.
        assert!(is_dry_run_push("git push -qn origin main"));
        assert!(is_dry_run_push("git push -nf"));
        assert!(is_dry_run_push("git push -vn origin HEAD"));
        // Neighbouring clusters without `n` are real pushes.
        assert!(!is_dry_run_push("git push -qf origin main"));
        assert!(!is_dry_run_push("git push -u origin fix/x"));
        // `-o <push-option>`: the rest of the cluster is its value, not flags.
        assert!(!is_dry_run_push(
            "git push -omerge.notify=false origin main"
        ));
    }

    #[test]
    fn dry_run_push_ignores_flags_that_are_not_the_pushs_own() {
        // A `-n` belonging to another command in the chain is not push's.
        assert!(!is_dry_run_push("echo -n x && git push"));
        assert!(!is_dry_run_push("git push && echo -n done"));
        // After `--`, tokens are refspecs rather than options.
        assert!(!is_dry_run_push("git push origin -- -n"));
        // A remote or branch *named* `push` does not shift the subcommand.
        assert!(is_dry_run_push("git push -n origin push"));
        assert!(!is_dry_run_push("git push origin push"));
        // Not a git push at all.
        assert!(!is_dry_run_push("gh pr create -n"));
    }

    #[test]
    fn is_git_push_with_config_override() {
        // `-c <name>=<value>` takes a separate value, like `-C <path>`.
        assert!(is_git_push("git -c push.default=simple push"));
        assert!(!is_git_push("git -c push.default=simple status"));
    }

    #[test]
    fn parse_git_push_failed_push_returns_none() {
        let out = "To https://github.com/o/r.git\n \
                   ! [rejected]        fix/x -> fix/x (non-fast-forward)\n\
                   error: failed to push some refs to 'https://github.com/o/r.git'\n";
        assert!(parse_git_push(out).is_none());

        // A rejected force-push prints a range-looking line only on the `!`
        // row, which is skipped — so no commit is claimed for it either.
        let out = "To https://github.com/o/r.git\n \
                   ! [remote rejected] fix/x -> fix/x (protected branch hook declined)\n";
        assert!(parse_git_push(out).is_none());
    }

    #[test]
    fn pushed_commit_is_claimed_and_recognised_on_the_sync_webhook() {
        // The whole point of reading the commit off the push output: the
        // abbreviated sha git printed has to recognise the full-length one the
        // `pull_request.synchronize` webhook carries.
        let out = "To https://github.com/o/r.git\n   2fa9788..7fc0612  fix/x -> fix/x\n";
        let commit = parse_git_push(out).unwrap().commit.unwrap();

        let claims = RecentlySent::new();
        assert!(claims.note_commit("task-1", &commit));
        assert!(claims.contains_commit("task-1", "7fc0612a1b2c3d4e5f60718293a4b5c6d7e8f901"));
    }

    #[test]
    fn parse_git_push_non_github_remote_returns_none() {
        let out = "To https://gitlab.com/o/r.git\n   abc..def  fix/x -> fix/x\n";
        assert!(parse_git_push(out).is_none());
    }

    #[test]
    fn parse_github_remote_https_strips_dot_git() {
        assert_eq!(
            parse_github_remote("https://github.com/soaplabs/soap-config.git"),
            Some(("soaplabs".into(), "soap-config".into()))
        );
        assert_eq!(
            parse_github_remote("https://github.com/o/r"),
            Some(("o".into(), "r".into()))
        );
    }

    #[test]
    fn parse_github_remote_ssh_forms() {
        assert_eq!(
            parse_github_remote("git@github.com:o/r.git"),
            Some(("o".into(), "r".into()))
        );
        assert_eq!(
            parse_github_remote("ssh://git@github.com/o/r.git"),
            Some(("o".into(), "r".into()))
        );
    }

    #[test]
    fn parse_github_remote_non_github_returns_none() {
        assert!(parse_github_remote("https://gitlab.com/o/r.git").is_none());
        assert!(parse_github_remote("not a url").is_none());
    }

    // ===== raw `gh api` classifier =====

    fn api_kind(cmd: &str) -> Option<GhApiCreated> {
        parse_gh_api_creation(cmd).map(|c| c.kind)
    }

    #[test]
    fn gh_api_creates_inline_review_comment() {
        // The shape the reviewer loop's prompt tells agents to use.
        assert_eq!(
            parse_gh_api_creation(
                "gh api --method POST repos/soaplabs/soap-config/pulls/42/comments \
                 -f body='needs a guard' -f commit_id=abc123 -f path=src/main.rs -F line=12"
            ),
            Some(GhApiCreation {
                owner: "soaplabs".into(),
                repo: "soap-config".into(),
                kind: GhApiCreated::ReviewComment,
            })
        );
        // Proxy prefix, rooted+quoted path, a quoted header, and the endpoint
        // sitting after the flags.
        assert_eq!(
            api_kind(
                "moat gh api -H \"Accept: application/vnd.github+json\" -X POST \
                 \"/repos/o/r/pulls/7/comments\" --input /tmp/comment.json"
            ),
            Some(GhApiCreated::ReviewComment)
        );
        // Chained prefix, implicit POST from `-f`, endpoint last.
        assert_eq!(
            api_kind("cd /tmp/repo && gh api -f body=hi repos/o/r/pulls/7/comments"),
            Some(GhApiCreated::ReviewComment)
        );
    }

    #[test]
    fn gh_api_submits_review() {
        assert_eq!(
            api_kind("gh api repos/o/r/pulls/42/reviews -f event=COMMENT -f body='LGTM'"),
            Some(GhApiCreated::Review)
        );
        assert_eq!(
            api_kind(
                "gh api --method=POST https://api.github.com/repos/o/r/pulls/42/reviews \
                 --input /tmp/review.json"
            ),
            Some(GhApiCreated::Review)
        );
        // Verbatim from the code-review loop's approve step, after the
        // message hook joins its backslash-newline continuation.
        let raw = "gh api repos/OWNER/REPO/pulls/42/reviews \\\n  \
                   --method POST --field event='APPROVE' --field body='Approved by Moatless'";
        assert_eq!(
            api_kind(normalize_line_continuations(raw).as_ref()),
            Some(GhApiCreated::Review)
        );
    }

    #[test]
    fn gh_api_creates_issue_comment() {
        assert_eq!(
            api_kind("gh api repos/o/r/issues/42/comments -f body='hi'"),
            Some(GhApiCreated::IssueComment)
        );
        // Env prefix, `-XPOST` glued form, a query string, and a quoted body
        // that names a *different* endpoint — quoting keeps it one token, so
        // it cannot be mistaken for the target.
        assert_eq!(
            parse_gh_api_creation(
                "GH_TOKEN=x gh api -f body=\"see repos/o/r/pulls/9/reviews for context\" \
                 -XPOST '/repos/o/r/issues/42/comments?foo=1'"
            ),
            Some(GhApiCreation {
                owner: "o".into(),
                repo: "r".into(),
                kind: GhApiCreated::IssueComment,
            })
        );
    }

    #[test]
    fn gh_api_read_only_calls_claim_nothing() {
        // No body flags — `gh api` defaults to GET.
        assert!(api_kind("gh api repos/o/r/pulls/42/comments").is_none());
        assert!(api_kind("gh api repos/o/r/issues/42/comments --paginate").is_none());
        // Explicit read, even with a body flag present.
        assert!(
            api_kind("gh api --method GET repos/o/r/pulls/42/reviews -f per_page=100").is_none()
        );
        assert!(api_kind("gh api -X GET repos/o/r/issues/42/comments").is_none());
    }

    #[test]
    fn gh_api_unrelated_resources_claim_nothing() {
        assert!(api_kind("gh api -X POST repos/o/r/issues/42/labels -f labels[]=bug").is_none());
        assert!(api_kind("gh api --method POST repos/o/r/pulls/42/merge").is_none());
        assert!(api_kind("gh api -X POST repos/o/r/issues -f title=bug").is_none());
        assert!(api_kind("gh api -X POST graphql -f query='{}'").is_none());
        // A non-numeric PR number is not an endpoint we recognise.
        assert!(api_kind("gh api -X POST repos/o/r/pulls/comments -f body=x").is_none());
        // Threaded replies go through the proxy and are out of scope here;
        // the deeper path must not be mistaken for a new inline comment.
        assert!(
            api_kind("gh api -X POST repos/o/r/pulls/42/comments/123/replies -f body=x").is_none()
        );
        // An edit reuses an existing id and produces no new event.
        assert!(api_kind("gh api -X PATCH repos/o/r/issues/42/comments -f body=x").is_none());
        // Not a `gh` invocation at all.
        assert!(api_kind("echo gh api -X POST repos/o/r/issues/42/comments").is_none());
        assert!(api_kind("gh pr comment 42 --body hi").is_none());
        assert!(api_kind("").is_none());
    }

    #[test]
    fn shell_tokens_respects_quoting_and_escapes() {
        assert_eq!(
            shell_tokens("gh api -H \"Accept: application/json\" -f body='a b'"),
            vec![
                "gh",
                "api",
                "-H",
                "Accept: application/json",
                "-f",
                "body=a b"
            ]
        );
        assert_eq!(
            shell_tokens("gh api -f body=a\\ b"),
            vec!["gh", "api", "-f", "body=a b"]
        );
    }

    // ===== created-id read-back =====

    #[test]
    fn created_id_read_from_json_response() {
        let body = r#"{
          "url": "https://api.github.com/repos/o/r/pulls/comments/987",
          "id": 987,
          "pull_request_review_id": 555,
          "html_url": "https://github.com/o/r/pull/42#discussion_r987"
        }"#;
        assert_eq!(
            parse_created_id(body, GhApiCreated::ReviewComment),
            Some(987)
        );
    }

    #[test]
    fn created_id_falls_back_to_html_url_anchor() {
        // Output the JSON parser cannot take — a preceding line from the same
        // shell command, or a truncated body.
        let review_comment =
            "posting…\n{\"id\": 987, \"html_url\": \"https://github.com/o/r/pull/42#discussion_r987\"";
        assert_eq!(
            parse_created_id(review_comment, GhApiCreated::ReviewComment),
            Some(987)
        );

        let review = "…\"html_url\":\"https://github.com/o/r/pull/42#pullrequestreview-555\",";
        assert_eq!(parse_created_id(review, GhApiCreated::Review), Some(555));

        let issue_comment = "…\"html_url\":\"https://github.com/o/r/issues/42#issuecomment-111\",";
        assert_eq!(
            parse_created_id(issue_comment, GhApiCreated::IssueComment),
            Some(111)
        );
    }

    #[test]
    fn created_id_absent_from_unusable_output() {
        // A list response is not an object, and carries no anchor for the
        // shape we asked about.
        assert!(parse_created_id("[{\"id\": 1}]", GhApiCreated::Review).is_none());
        assert!(parse_created_id("", GhApiCreated::IssueComment).is_none());
        assert!(
            parse_created_id("gh: Not Found (HTTP 404)", GhApiCreated::ReviewComment).is_none()
        );
        // The anchor must match the shape — a review submission's response
        // does not name a review comment.
        assert!(parse_created_id(
            "\"html_url\":\"https://github.com/o/r/pull/42#pullrequestreview-555\"",
            GhApiCreated::ReviewComment
        )
        .is_none());
    }

    #[test]
    fn claimed_id_comes_from_the_response_not_the_request() {
        // The endpoint names PR 42; the claim names comment 987, which only
        // the response can supply.
        let created = parse_gh_api_creation(
            "gh api -X POST repos/o/r/pulls/42/comments -f body=x -f path=a.rs -F line=1",
        )
        .unwrap();
        let id = parse_created_id(
            r#"{"id": 987, "pull_request_review_id": 555}"#,
            created.kind,
        )
        .unwrap();
        assert_eq!(created.kind.external_id(id), "github:rc:987");
    }

    #[test]
    fn each_api_shape_claims_its_own_external_id_namespace() {
        assert_eq!(GhApiCreated::ReviewComment.external_id(1), "github:rc:1");
        assert_eq!(GhApiCreated::Review.external_id(2), "github:rv:2");
        assert_eq!(GhApiCreated::IssueComment.external_id(3), "github:ic:3");
    }
}
