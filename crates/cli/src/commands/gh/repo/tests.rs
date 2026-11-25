//! Which repository an argument or an `origin` remote names.

use super::*;

#[test]
fn parses_pull_request_urls() {
    assert_eq!(
        parse_pull_request_url("https://github.com/soaplabs/moatless/pull/208"),
        Some(PullRequestRef {
            owner: "soaplabs".into(),
            repo: "moatless".into(),
            number: 208,
        })
    );
    assert_eq!(
        parse_pull_request_url("https://github.com/soaplabs/moatless/issues/208"),
        None
    );
    assert_eq!(parse_pull_request_url("not-a-url"), None);
}

#[test]
fn parse_repo_arg_accepts_slug_and_urls() {
    assert_eq!(parse_repo_arg("o/r"), Some(("o".into(), "r".into())));
    assert_eq!(
        parse_repo_arg("https://github.com/o/r.git"),
        Some(("o".into(), "r".into()))
    );
    assert_eq!(
        parse_repo_arg("git@github.com:o/r.git"),
        Some(("o".into(), "r".into()))
    );
    assert_eq!(parse_repo_arg("just-a-name"), None);
    assert_eq!(parse_repo_arg("a/b/c"), None);
    assert_eq!(parse_repo_arg("/r"), None);
}

#[test]
fn parse_github_remote_forms() {
    assert_eq!(
        parse_github_remote("https://github.com/soaplabs/soap-backend.git"),
        Some(("soaplabs".into(), "soap-backend".into()))
    );
    assert_eq!(
        parse_github_remote("ssh://git@github.com/o/r.git"),
        Some(("o".into(), "r".into()))
    );
    assert_eq!(
        parse_github_remote("git+https://github.com/o/r.git"),
        Some(("o".into(), "r".into()))
    );
    assert_eq!(parse_github_remote("https://gitlab.com/o/r.git"), None);
}

#[test]
fn a_proxied_origin_is_recognized_whichever_url_names_the_backend() {
    assert_eq!(
        any_host_proxy_repository_id(
            "http://moatless-backend.moatless.svc.cluster.local:8080/git/repo_1"
        ),
        Some("repo_1")
    );
    assert_eq!(
        any_host_proxy_repository_id("https://moatless.example.com/git/repo_1.git"),
        Some("repo_1")
    );
    assert_eq!(
        any_host_proxy_repository_id("https://github.com/soaplabs/moatless.git"),
        None,
        "a real git host names an owner, not a repository id"
    );
    assert_eq!(any_host_proxy_repository_id("https://host/git/"), None);
}

/// A sandbox clones over HTTPS with its token in the remote, so this is what
/// `git remote get-url origin` answers in most checkouts `moat gh` runs in.
#[test]
fn a_credential_in_the_remote_neither_blocks_the_match_nor_reaches_the_message() {
    let cloned = "https://x-access-token:ghu_SECRET@github.com/soaplabs/soap-knowledge";
    assert_eq!(
        parse_github_remote(&strip_userinfo(cloned)),
        Some(("soaplabs".into(), "soap-knowledge".into()))
    );
    assert_eq!(
        redact_userinfo(cloned),
        "https://***@github.com/soaplabs/soap-knowledge"
    );
    assert!(!redact_userinfo(cloned).contains("ghu_SECRET"));

    assert_eq!(
        strip_userinfo("ssh://git@github.com/o/r.git"),
        "ssh://git@github.com/o/r.git",
        "the ssh form's `git@` is a username, not a credential, and parse_github_remote wants it"
    );
    assert_eq!(
        strip_userinfo("https://github.com/o/r.git"),
        "https://github.com/o/r.git"
    );
    assert_eq!(
        strip_userinfo("https://user:pw@host:8080/git/repo_1"),
        "https://host:8080/git/repo_1",
        "a port survives: the `@` is found within the authority, not past the first slash"
    );
}

/// The backend a proxied origin is cloned from.
const BACKEND: &str = "https://moatless.example.com";

/// `gh` left to itself prefers a remote named `upstream`, so in a fork clone
/// that has both, this is the difference between reading the fork's PR and
/// reading whatever upstream PR shares its number.
#[test]
fn a_fork_s_origin_names_the_fork_whatever_else_it_has_a_remote_for() {
    assert!(matches!(
        classify_origin(BACKEND, "https://github.com/soaplabs/t3code.git"),
        OriginUrl::Github(owner, name) if owner == "soaplabs" && name == "t3code"
    ));
}

#[test]
fn a_proxied_origin_names_the_id_to_read_back() {
    assert!(matches!(
        classify_origin(BACKEND, "https://moatless.example.com/git/repo_1"),
        OriginUrl::Proxied("repo_1")
    ));
}

#[test]
fn an_origin_on_another_host_names_no_repository() {
    assert!(matches!(
        classify_origin(BACKEND, "https://forgejo.example.com/o/r.git"),
        OriginUrl::Foreign
    ));
}

fn target(explicit: bool) -> RepoTarget {
    RepoTarget {
        owner: "soaplabs".to_string(),
        repo: "moatless".to_string(),
        explicit,
    }
}

/// A 404 on its own reads as a pull request that does not exist. The repository
/// the call addressed, and where that name came from, is what tells the caller
/// the cwd is a different repository from the pull request's.
#[test]
fn a_failed_write_names_the_repository_it_addressed() {
    let inferred = target(false).failed("#1318", "GitHub post_comment returned 404 Not Found");
    let message = inferred.to_string();
    assert!(
        message.contains("GitHub post_comment returned 404"),
        "{message}"
    );
    assert!(message.contains("soaplabs/moatless#1318"), "{message}");
    assert!(
        message.contains("inferred from the `origin` remote of the current directory"),
        "{message}"
    );

    let named = target(true).failed("#1318", "boom").to_string();
    assert!(named.contains("named by `-R`"), "{named}");
}

/// `pr create` names no number, so the repository stands alone in its message.
#[test]
fn a_failed_pr_create_names_the_repository_without_a_number() {
    let message = target(false).failed("", "boom").to_string();
    assert!(
        message.contains("addressed soaplabs/moatless,"),
        "{message}"
    );
}
