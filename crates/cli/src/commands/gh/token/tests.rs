//! Which credential source wins, and what a failed read falls back to.

use super::*;

/// The reported 401: `GH_TOKEN` is seeded at provision and expires with the
/// installation token behind it, so in a sandbox the broker's per-call token
/// has to outrank it.
#[test]
fn the_broker_outranks_a_stale_env_token() {
    assert_eq!(
        resolve_gh_token(true, true, || Some("fresh".to_string())),
        Some("fresh".to_string())
    );
}

#[test]
fn an_exported_env_token_still_wins_off_the_broker() {
    assert_eq!(
        resolve_gh_token(false, true, || Some("helper".to_string())),
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

/// A broker read that fails leaves `gh` to find the ambient `GH_TOKEN`, which
/// beats sending the call unauthenticated.
#[test]
fn a_failed_broker_read_falls_back_to_gh_s_own_resolution() {
    assert_eq!(resolve_gh_token(true, true, || None), None);
}
