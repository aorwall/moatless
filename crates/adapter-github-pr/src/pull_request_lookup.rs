//! Resolve provider pull requests independently of HTTP.
use super::adapter::GithubPrAdapter;
use super::auth::resolve_github_api;
use moatless_adapter_model::error::AdapterError;

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
}
