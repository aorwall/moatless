//! Which REST writes a sandbox may send.

use moatless_core::proxy_rules::NormalPath;
use moatless_core::workspace_identity_grants::GitHubCapability;
use moatless_workspace_types::repository::GITHUB_HOST;

use super::refusal::{Refusal, capability_rule, review_needs_approve};
use super::scope::{PolicyScope, repo_key};

/// What an allowed write needs beyond an allowed repository.
#[derive(Clone, Copy)]
enum Need {
    Nothing,
    /// A review whose body `event` is `APPROVE` needs
    /// [`GitHubCapability::Approve`].
    Review,
    Capability(GitHubCapability),
}

/// The writes allowed under `/repos/{owner}/{name}/`, each a method set and
/// the segments after the name, where `*` is one segment. Anything else under
/// a repository (contents, git data, releases, settings, hooks, keys,
/// secrets, forks, collaborators) is refused.
const WRITES: &[(&[&str], &str, Need)] = &[
    (&["POST"], "pulls", Need::Nothing),
    (&["PATCH"], "pulls/*", Need::Nothing),
    (&["PUT"], "pulls/*/update-branch", Need::Nothing),
    (
        &["POST", "DELETE"],
        "pulls/*/requested_reviewers",
        Need::Nothing,
    ),
    (&["POST"], "pulls/*/comments", Need::Nothing),
    (&["POST"], "pulls/*/comments/*/replies", Need::Nothing),
    (&["PATCH", "DELETE"], "pulls/comments/*", Need::Nothing),
    (&["POST"], "pulls/comments/*/reactions", Need::Nothing),
    (&["DELETE"], "pulls/comments/*/reactions/*", Need::Nothing),
    (&["POST"], "pulls/*/reviews", Need::Review),
    (&["POST"], "pulls/*/reviews/*/events", Need::Review),
    (&["PUT", "DELETE"], "pulls/*/reviews/*", Need::Nothing),
    (
        &["PUT"],
        "pulls/*/merge",
        Need::Capability(GitHubCapability::Merge),
    ),
    (&["POST"], "issues", Need::Nothing),
    (&["PATCH"], "issues/*", Need::Nothing),
    (&["POST"], "issues/*/comments", Need::Nothing),
    (&["PATCH", "DELETE"], "issues/comments/*", Need::Nothing),
    (&["POST"], "issues/comments/*/reactions", Need::Nothing),
    (&["DELETE"], "issues/comments/*/reactions/*", Need::Nothing),
    (&["POST", "PUT", "DELETE"], "issues/*/labels", Need::Nothing),
    (&["DELETE"], "issues/*/labels/*", Need::Nothing),
    (&["POST", "DELETE"], "issues/*/assignees", Need::Nothing),
    (&["POST"], "issues/*/reactions", Need::Nothing),
    (&["DELETE"], "issues/*/reactions/*", Need::Nothing),
    (&["POST"], "statuses/*", Need::Nothing),
    (&["POST"], "check-runs", Need::Nothing),
    (&["PATCH"], "check-runs/*", Need::Nothing),
    (&["POST"], "check-runs/*/rerequest", Need::Nothing),
    (&["POST"], "check-suites/*/rerequest", Need::Nothing),
    (&["POST"], "actions/runs/*/rerun", Need::Nothing),
    (&["POST"], "actions/runs/*/rerun-failed-jobs", Need::Nothing),
    (&["POST"], "actions/runs/*/cancel", Need::Nothing),
    (&["POST"], "actions/jobs/*/rerun", Need::Nothing),
    (&["POST"], "actions/workflows/*/dispatches", Need::Nothing),
];

pub(super) fn is_read(method: &str) -> bool {
    matches!(method, "GET" | "HEAD" | "OPTIONS")
}

/// `/repos/{owner}/{name}/{rest…}` split, or `None` outside `/repos/`.
fn repository_path(path: &NormalPath) -> Option<(&str, &str, Vec<&str>)> {
    let mut segments = path.as_str().strip_prefix("/repos/")?.split('/');
    let owner = segments.next().filter(|s| !s.is_empty())?;
    let name = segments.next().filter(|s| !s.is_empty())?;
    Some((owner, name, segments.filter(|s| !s.is_empty()).collect()))
}

fn segments_match(pattern: &str, rest: &[&str]) -> bool {
    let pattern: Vec<&str> = pattern.split('/').collect();
    pattern.len() == rest.len() && pattern.iter().zip(rest).all(|(p, s)| *p == "*" || p == s)
}

fn rule_for(method: &str, rest: &[&str]) -> Option<(&'static str, Need)> {
    WRITES
        .iter()
        .find(|(methods, pattern, _)| methods.contains(&method) && segments_match(pattern, rest))
        .map(|(_, pattern, need)| (*pattern, *need))
}

/// Whether [`check`] reads the body of this request.
pub(super) fn needs_body(method: &str, path: &NormalPath) -> bool {
    repository_path(path)
        .and_then(|(_, _, rest)| rule_for(method, &rest))
        .is_some_and(|(_, need)| matches!(need, Need::Review))
}

pub(super) fn check(
    scope: &PolicyScope,
    method: &str,
    path: &NormalPath,
    body: Option<&[u8]>,
) -> Result<(), Refusal> {
    let Some((owner, name, rest)) = repository_path(path) else {
        return Err(Refusal::new(
            "outside_repos",
            format!(
                "The GitHub proxy allows no {method} outside /repos/: a sandbox writes only to \
                 its workspace's repositories"
            ),
        ));
    };
    let slug = format!("{owner}/{name}");
    if !scope.allows(&repo_key(GITHUB_HOST, owner, name)) {
        return Err(Refusal::new(
            "repo_scope",
            format!(
                "{slug} is not one of this workspace's repositories: place it in the workspace, \
                 or add it to `githubPolicy.extraRepos` in its WORKSPACE_IDENTITY_GRANTS entry"
            ),
        ));
    }
    let Some((pattern, need)) = rule_for(method, &rest) else {
        return Err(Refusal::new(
            "denied_write",
            format!(
                "The GitHub proxy allows no {method} {}: a sandbox may not change a \
                 repository's contents, settings or releases through the API",
                path.as_str()
            ),
        ));
    };
    match need {
        Need::Nothing => Ok(()),
        Need::Review if !review_needs_approve(body) => Ok(()),
        Need::Review => require(scope, GitHubCapability::Approve, pattern),
        Need::Capability(capability) => require(scope, capability, pattern),
    }
}

fn require(scope: &PolicyScope, capability: GitHubCapability, what: &str) -> Result<(), Refusal> {
    if scope.has(capability) {
        return Ok(());
    }
    Err(Refusal::new(
        capability_rule(capability),
        format!(
            "{what} needs the `{}` capability in `githubPolicy.capabilities` of this \
             workspace's WORKSPACE_IDENTITY_GRANTS entry",
            capability.as_str()
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github_policy::test_support::scope;

    fn decide(
        capabilities: &[GitHubCapability],
        method: &str,
        path: &str,
        body: Option<&str>,
    ) -> Result<(), &'static str> {
        let path = NormalPath::parse(path).unwrap();
        check(&scope(capabilities), method, &path, body.map(str::as_bytes)).map_err(|r| r.rule)
    }

    #[test]
    fn pull_request_and_issue_work_is_allowed_on_a_placed_repository() {
        for (method, path) in [
            ("POST", "/repos/soaplabs/moatless/pulls"),
            ("PATCH", "/repos/soaplabs/moatless/pulls/7"),
            (
                "POST",
                "/repos/soaplabs/moatless/pulls/7/comments/9/replies",
            ),
            ("POST", "/repos/soaplabs/moatless/issues/7/comments"),
            ("DELETE", "/repos/soaplabs/moatless/issues/7/labels/routine"),
            (
                "POST",
                "/repos/soaplabs/extra/actions/workflows/build.yml/dispatches",
            ),
            ("POST", "/repos/soaplabs/moatless/statuses/abc123"),
        ] {
            assert_eq!(decide(&[], method, path, None), Ok(()), "{method} {path}");
        }
    }

    #[test]
    fn a_write_to_another_repository_or_outside_repos_is_refused() {
        assert_eq!(
            decide(&[], "POST", "/repos/soaplabs/other/issues", None),
            Err("repo_scope")
        );
        assert_eq!(
            decide(&[], "POST", "/user/repos", None),
            Err("outside_repos")
        );
        assert_eq!(decide(&[], "POST", "/gists", None), Err("outside_repos"));
    }

    #[test]
    fn contents_git_data_settings_and_releases_are_refused_even_on_a_placed_repository() {
        for (method, path) in [
            (
                "PUT",
                "/repos/soaplabs/moatless/contents/.github/workflows/ci.yml",
            ),
            ("POST", "/repos/soaplabs/moatless/git/refs"),
            ("PATCH", "/repos/soaplabs/moatless"),
            ("DELETE", "/repos/soaplabs/moatless"),
            ("POST", "/repos/soaplabs/moatless/releases"),
            ("PUT", "/repos/soaplabs/moatless/branches/main/protection"),
            ("PUT", "/repos/soaplabs/moatless/actions/secrets/TOKEN"),
            ("POST", "/repos/soaplabs/moatless/hooks"),
            ("POST", "/repos/soaplabs/moatless/forks"),
        ] {
            assert_eq!(
                decide(&[GitHubCapability::Merge], method, path, None),
                Err("denied_write"),
                "{method} {path}"
            );
        }
    }

    #[test]
    fn an_approving_review_needs_the_approve_capability() {
        let path = "/repos/soaplabs/moatless/pulls/7/reviews";
        let approve = Some(r#"{"event":"APPROVE","body":"lgtm"}"#);
        assert_eq!(decide(&[], "POST", path, approve), Err("approve"));
        assert_eq!(
            decide(&[GitHubCapability::Approve], "POST", path, approve),
            Ok(())
        );
        assert_eq!(
            decide(&[], "POST", path, Some(r#"{"event":"COMMENT"}"#)),
            Ok(())
        );
        assert_eq!(decide(&[], "POST", path, Some(r#"{"body":"x"}"#)), Ok(()));
        assert_eq!(decide(&[], "POST", path, Some("not json")), Err("approve"));
        let submit = "/repos/soaplabs/moatless/pulls/7/reviews/3/events";
        assert_eq!(decide(&[], "POST", submit, approve), Err("approve"));
    }

    #[test]
    fn a_merge_needs_the_merge_capability() {
        let path = "/repos/soaplabs/moatless/pulls/7/merge";
        assert_eq!(decide(&[], "PUT", path, None), Err("merge"));
        assert_eq!(
            decide(&[GitHubCapability::Merge], "PUT", path, None),
            Ok(())
        );
    }

    #[test]
    fn only_a_review_reads_its_body() {
        let review = NormalPath::parse("/repos/a/b/pulls/1/reviews").unwrap();
        let comment = NormalPath::parse("/repos/a/b/issues/1/comments").unwrap();
        assert!(needs_body("POST", &review));
        assert!(!needs_body("POST", &comment));
    }
}
