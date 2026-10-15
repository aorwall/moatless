//! Resolve provider pull requests independently of HTTP.
use super::adapter::GithubPrAdapter;
use super::auth::resolve_github_api;
use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_types::pull_request_conversation::{FileChangeType, FileContentsPair};

/// A pull request resolved for the T3 attach-PR dialog and the creation-time
/// binding, in the shape T3's `GitResolvedPullRequest` reports.
pub struct DialogPullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub base_ref: String,
    pub head_ref: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
}

/// Resolve an attach-PR reference to the pull request it names.
///
/// Follows an `owner/repo#N` or PR-URL reference against the cwd's
/// `repo_owner/repo_name`; `Ok(None)` when the reference is not one this
/// resolver understands.
pub async fn resolve_reference_for_dialog(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    repo_owner: &str,
    repo_name: &str,
    reference: &str,
) -> Result<Option<DialogPullRequest>, AdapterError> {
    let Some((owner, repo, number)) =
        parse_pull_request_reference(reference, repo_owner, repo_name)
    else {
        return Ok(None);
    };
    let api = resolve_github_api(adapter, user_id, workspace_id, &owner, &repo).await?;
    let pr = api.get_pr(&owner, &repo, number).await?;
    Ok(Some(to_dialog_pull_request(&pr, &owner, &repo)))
}

/// The open pull request whose head branch is `head`, for binding a Task
/// created on a PR's head branch.
pub async fn find_open_pull_request_for_head(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    owner: &str,
    repo: &str,
    head: &str,
) -> Result<Option<DialogPullRequest>, AdapterError> {
    let api = resolve_github_api(adapter, user_id, workspace_id, owner, repo).await?;
    Ok(api
        .find_open_pr_for_head(owner, repo, head)
        .await?
        .map(|pr| to_dialog_pull_request(&pr, owner, repo)))
}

/// A pull request as the T3 review panel shows it.
pub struct PullRequestOverview {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub url: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
    pub is_draft: bool,
    pub author_login: Option<String>,
    pub author_avatar_url: Option<String>,
    pub additions: u64,
    pub deletions: u64,
    pub changed_files: u64,
    pub head_ref: String,
    pub head_sha: String,
    /// `owner/repo` of the head branch, when that repository still exists.
    pub head_repository: Option<String>,
    pub base_ref: String,
    pub base_sha: String,
    /// GitHub's timestamps, RFC 3339.
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub merged_at: Option<String>,
    pub closed_at: Option<String>,
    pub mergeable: Option<bool>,
    pub mergeable_state: Option<String>,
}

/// Read `owner/repo#number` with the caller's GitHub credentials.
pub async fn pull_request_overview(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    owner: &str,
    repo: &str,
    number: u64,
) -> Result<PullRequestOverview, AdapterError> {
    let api = resolve_github_api(adapter, user_id, workspace_id, owner, repo).await?;
    let pr = api.get_pr(owner, repo, number).await?;
    let dialog = to_dialog_pull_request(&pr, owner, repo);
    Ok(PullRequestOverview {
        number: pr.number,
        title: dialog.title,
        body: pr.body.unwrap_or_default(),
        url: dialog.url,
        state: dialog.state,
        is_draft: pr.draft.unwrap_or(false),
        author_login: pr.user.as_ref().map(|user| user.login.clone()),
        author_avatar_url: pr.user.and_then(|user| user.avatar_url),
        additions: pr.additions.unwrap_or_default(),
        deletions: pr.deletions.unwrap_or_default(),
        changed_files: pr.changed_files.unwrap_or_default(),
        head_ref: pr.head.ref_name,
        head_sha: pr.head.sha,
        head_repository: pr.head.repo.map(|repo| repo.full_name),
        base_ref: pr.base.ref_name,
        base_sha: pr.base.sha,
        created_at: pr.created_at,
        updated_at: pr.updated_at,
        merged_at: pr.merged_at,
        closed_at: pr.closed_at,
        mergeable: pr.mergeable,
        mergeable_state: pr.mergeable_state,
    })
}

/// Files per page of [`pull_request_patch_page`]: GitHub's largest page.
const FILES_PER_PAGE: u32 = 100;
/// GitHub lists at most 3000 files of a pull request.
const MAX_FILE_PAGES: u32 = 3000 / FILES_PER_PAGE;

/// One page of a pull request's change as a git-style patch.
pub struct PullRequestPatchPage {
    pub patch: String,
    /// The page to ask for next, `None` on the last one.
    pub next_page: Option<u32>,
    /// Files on this page GitHub would not diff (binary or too large), which
    /// the patch leaves out.
    pub omitted_files: usize,
}

/// Page `page` (from 1) of `owner/repo#number`'s changed files, rebuilt into
/// the `diff --git` patch T3's diff parser reads.
pub async fn pull_request_patch_page(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    owner: &str,
    repo: &str,
    number: u64,
    page: u32,
) -> Result<PullRequestPatchPage, AdapterError> {
    let api = resolve_github_api(adapter, user_id, workspace_id, owner, repo).await?;
    let page = page.max(1);
    let files = api
        .list_pr_files(owner, repo, number, page, FILES_PER_PAGE)
        .await?;
    Ok(patch_page(&files, page))
}

/// [`pull_request_patch_page`] for the one commit `sha` of `owner/repo`.
pub async fn commit_patch_page(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    owner: &str,
    repo: &str,
    sha: &str,
    page: u32,
) -> Result<PullRequestPatchPage, AdapterError> {
    let api = resolve_github_api(adapter, user_id, workspace_id, owner, repo).await?;
    let page = page.max(1);
    let commit = api
        .get_commit(owner, repo, sha, page, FILES_PER_PAGE)
        .await?;
    Ok(patch_page(&commit.files, page))
}

fn patch_page(files: &[super::api::PrFile], page: u32) -> PullRequestPatchPage {
    let full = u32::try_from(files.len()).unwrap_or(u32::MAX) >= FILES_PER_PAGE;
    let mut patch = String::new();
    let mut omitted_files = 0;
    for file in files {
        if !push_file_patch(&mut patch, file) {
            omitted_files += 1;
        }
    }
    PullRequestPatchPage {
        patch,
        next_page: (full && page < MAX_FILE_PAGES).then_some(page + 1),
        omitted_files,
    }
}

/// Which version of a pull request, or of one of its commits, a file is read
/// at.
pub enum ContentsScope<'a> {
    /// The whole change: the merge base of its base and head against its
    /// head, the two sides GitHub's patch was taken between.
    PullRequest(u64),
    /// One commit against its first parent.
    Commit(&'a str),
}

/// A changed file before and after the change `scope` names, `old_path` read
/// at the older revision and `new_path` at the newer one. The side `change`
/// says the file does not exist on is empty.
#[expect(
    clippy::too_many_arguments,
    reason = "each argument is one fact of a single read; a struct for them would have one caller"
)]
pub async fn file_contents(
    adapter: &GithubPrAdapter,
    user_id: &str,
    workspace_id: Option<&str>,
    owner: &str,
    repo: &str,
    scope: ContentsScope<'_>,
    change: FileChangeType,
    old_path: &str,
    new_path: &str,
) -> Result<FileContentsPair, AdapterError> {
    let api = resolve_github_api(adapter, user_id, workspace_id, owner, repo).await?;
    // A root commit has no parent, so only a file it adds can be shown.
    let (old_revision, new_revision) = match scope {
        ContentsScope::PullRequest(number) => {
            let pr = api.get_pr(owner, repo, number).await?;
            // Deliberately not `base.sha`: once the base branch has moved on,
            // its tip holds lines the hunks' left side never had.
            let base = api
                .merge_base(owner, repo, &pr.base.sha, &pr.head.sha)
                .await?;
            (Some(base), pr.head.sha)
        }
        ContentsScope::Commit(sha) => {
            let commit = api.get_commit(owner, repo, sha, 1, 1).await?;
            (
                commit.parents.into_iter().next().map(|parent| parent.sha),
                commit.sha,
            )
        }
    };
    let old_contents = if change == FileChangeType::New {
        String::new()
    } else {
        let revision = old_revision.ok_or_else(|| {
            AdapterError::from(SharedAdapterError::AdapterApi(format!(
                "{old_path} has no earlier version in a root commit"
            )))
        })?;
        api.file_text(owner, repo, old_path, &revision).await?
    };
    let new_contents = match change {
        FileChangeType::Deleted => String::new(),
        _ => api.file_text(owner, repo, new_path, &new_revision).await?,
    };
    Ok(FileContentsPair {
        old_contents,
        new_contents,
    })
}

/// Append `file` to `patch` with the headers `git diff` would have written.
/// False when GitHub gave no hunks to append.
fn push_file_patch(patch: &mut String, file: &super::api::PrFile) -> bool {
    let Some(hunks) = file.patch.as_deref().filter(|hunks| !hunks.is_empty()) else {
        return false;
    };
    let new = file.filename.as_str();
    let old = file.previous_filename.as_deref().unwrap_or(new);
    let headers = match file.status.as_str() {
        "added" => format!("new file mode 100644\n--- /dev/null\n+++ b/{new}\n"),
        "removed" => format!("deleted file mode 100644\n--- a/{old}\n+++ /dev/null\n"),
        _ if old != new => {
            format!("rename from {old}\nrename to {new}\n--- a/{old}\n+++ b/{new}\n")
        }
        _ => format!("--- a/{old}\n+++ b/{new}\n"),
    };
    patch.push_str("diff --git a/");
    patch.push_str(old);
    patch.push_str(" b/");
    patch.push_str(new);
    patch.push('\n');
    patch.push_str(&headers);
    patch.push_str(hunks);
    if !hunks.ends_with('\n') {
        patch.push('\n');
    }
    true
}

/// Parse `N`, `owner/repo#N`, or a `github.com/owner/repo/pull/N` URL into the
/// repository and number to resolve. A bare number names the cwd's repository.
fn parse_pull_request_reference(
    reference: &str,
    default_owner: &str,
    default_repo: &str,
) -> Option<(String, String, u64)> {
    let reference = reference.trim();
    if let Ok(number) = reference.parse::<u64>() {
        return Some((default_owner.to_string(), default_repo.to_string(), number));
    }
    if let Some((repo_part, number_part)) = reference.split_once('#') {
        let (owner, repo) = repo_part.trim().split_once('/')?;
        let number = number_part.trim().parse().ok()?;
        let (owner, repo) = (owner.trim(), repo.trim());
        if owner.is_empty() || repo.is_empty() {
            return None;
        }
        return Some((owner.to_string(), repo.to_string(), number));
    }
    // A full pull-request URL: `.../{owner}/{repo}/pull/{number}`.
    let path = reference.split("github.com/").nth(1)?.trim_end_matches('/');
    let mut segments = path.split('/');
    let owner = segments.next()?.trim();
    let repo = segments.next()?.trim();
    if segments.next()? != "pull" {
        return None;
    }
    let number = segments.next()?.trim().parse().ok()?;
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_string(), repo.to_string(), number))
}

fn to_dialog_pull_request(pr: &super::api::PrInfo, owner: &str, repo: &str) -> DialogPullRequest {
    let state = if pr.merged == Some(true) {
        "merged"
    } else if pr.state.as_str() == "closed" {
        "closed"
    } else {
        "open"
    };
    let url = pr
        .html_url
        .clone()
        .unwrap_or_else(|| format!("https://github.com/{owner}/{repo}/pull/{}", pr.number));
    let title = if pr.title.trim().is_empty() {
        format!("#{}", pr.number)
    } else {
        pr.title.clone()
    };
    DialogPullRequest {
        number: pr.number,
        title,
        url,
        base_ref: pr.base.ref_name.clone(),
        head_ref: pr.head.ref_name.clone(),
        state: state.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_keep_their_repository_scope() {
        for (input, owner, repo) in [
            (" 42 ", "default", "repo"),
            ("other/project#42", "other", "project"),
            (
                "https://github.com/other/project/pull/42/",
                "other",
                "project",
            ),
        ] {
            assert_eq!(
                parse_pull_request_reference(input, "default", "repo"),
                Some((owner.into(), repo.into(), 42))
            );
        }
        for input in ["feature/foo", "/repo#42", "owner/#42", "owner/repo#bad"] {
            assert_eq!(parse_pull_request_reference(input, "default", "repo"), None);
        }
    }

    #[test]
    fn merged_state_and_missing_display_fields_keep_their_fallbacks() {
        let pr = serde_json::from_value(serde_json::json!({
            "number": 42,
            "title": "  ",
            "state": "closed",
            "merged": true,
            "head": {"ref": "feature", "sha": "abc"},
            "base": {"ref": "main", "sha": "def"}
        }))
        .unwrap();
        let resolved = to_dialog_pull_request(&pr, "owner", "repo");
        assert_eq!(resolved.state, "merged");
        assert_eq!(resolved.title, "#42");
        assert_eq!(resolved.url, "https://github.com/owner/repo/pull/42");
        assert_eq!(resolved.head_ref, "feature");
        assert_eq!(resolved.base_ref, "main");
    }

    fn file(
        status: &str,
        filename: &str,
        previous: Option<&str>,
        patch: Option<&str>,
    ) -> super::super::api::PrFile {
        serde_json::from_value(serde_json::json!({
            "filename": filename,
            "previous_filename": previous,
            "status": status,
            "patch": patch,
        }))
        .unwrap()
    }

    #[test]
    fn file_patches_get_the_headers_git_writes() {
        let mut patch = String::new();
        assert!(push_file_patch(
            &mut patch,
            &file("modified", "a.rs", None, Some("@@ -1 +1 @@\n-a\n+b"))
        ));
        assert!(push_file_patch(
            &mut patch,
            &file("added", "n.rs", None, Some("@@ -0,0 +1 @@\n+n\n"))
        ));
        assert!(push_file_patch(
            &mut patch,
            &file(
                "renamed",
                "new.rs",
                Some("old.rs"),
                Some("@@ -1 +1 @@\n-x\n+y")
            )
        ));
        assert!(!push_file_patch(
            &mut patch,
            &file("modified", "logo.png", None, None)
        ));
        assert_eq!(
            patch,
            "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1 +1 @@\n-a\n+b\n\
             diff --git a/n.rs b/n.rs\nnew file mode 100644\n--- /dev/null\n+++ b/n.rs\n@@ -0,0 +1 @@\n+n\n\
             diff --git a/old.rs b/new.rs\nrename from old.rs\nrename to new.rs\n--- a/old.rs\n+++ b/new.rs\n@@ -1 +1 @@\n-x\n+y\n"
        );
    }
}
