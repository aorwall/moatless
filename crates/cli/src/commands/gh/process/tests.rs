//! The exit status a child's failure travels on, and the environment it runs
//! with.

use super::*;

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(ToString::to_string).collect()
}

#[test]
fn github_child_status_remains_available_to_the_executable() {
    assert!(command_status(0).is_ok());
    for code in [1, 2, 42, 127] {
        let error = command_status(code).expect_err("nonzero child status");
        let status = error
            .downcast_ref::<CommandExit>()
            .expect("typed child status");
        assert_eq!(status.0, code);
    }
}

fn gh_repo_env(gh_repo: Option<&str>) -> Option<String> {
    gh_command(&argv(&["pr", "view"]), None, gh_repo)
        .get_envs()
        .find(|(key, _)| *key == std::ffi::OsStr::new("GH_REPO"))
        .and_then(|(_, value)| value)
        .map(|value| value.to_string_lossy().into_owned())
}

/// Without this, `gh` resolves the repository itself: it prefers an `upstream`
/// remote over `origin`, and a proxied origin names no owner at all.
#[test]
fn the_resolved_repository_reaches_gh_as_gh_repo() {
    assert_eq!(
        gh_repo_env(Some("owner/repo")).as_deref(),
        Some("owner/repo")
    );
}

#[test]
fn an_origin_that_resolved_to_nothing_leaves_the_environment_alone() {
    assert_eq!(gh_repo_env(None), None);
}
