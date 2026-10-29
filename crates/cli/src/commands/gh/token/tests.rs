//! Which credential source wins.

use super::*;

#[test]
fn an_exported_env_token_wins_off_the_proxy() {
    assert_eq!(
        resolve_gh_token(false, true, || panic!("no token is fetched")),
        None
    );
}

#[test]
fn the_credential_helper_answers_when_no_env_token_is_set() {
    assert_eq!(
        resolve_gh_token(false, false, || Some("helper".to_string())),
        Some("helper".to_string())
    );
}

/// Behind the proxy no token is fetched at all: the pod's `gh` never holds one,
/// and the backend presents the live token per request.
#[test]
fn behind_the_github_proxy_gh_is_handed_no_token() {
    for env_token_set in [false, true] {
        assert_eq!(
            resolve_gh_token(true, env_token_set, || panic!("no token is fetched")),
            None
        );
    }
}
