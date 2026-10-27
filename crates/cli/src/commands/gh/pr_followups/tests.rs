//! Checking what GitHub answered against the labels and reviewers asked for.

use super::*;

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(ToString::to_string).collect()
}

#[test]
fn a_label_github_lists_back_has_landed() {
    let body = br#"[{"id":1,"name":"Bug"},{"id":2,"name":"agent-rule-enforcement"}]"#;
    assert!(
        labels_missing_from(body, &argv(&["bug", "agent-rule-enforcement"])).is_empty(),
        "GitHub matches a label name case-insensitively"
    );
}

#[test]
fn a_label_absent_from_the_answer_is_reported_missing() {
    let body = br#"[{"id":1,"name":"bug"}]"#;
    assert_eq!(
        labels_missing_from(body, &argv(&["bug", "needs-triage"])),
        vec!["needs-triage".to_string()]
    );
    // An unreadable answer confirms nothing, so nothing counts as applied.
    assert_eq!(
        labels_missing_from(b"not json", &argv(&["bug"])),
        vec!["bug".to_string()]
    );
}

#[test]
fn a_label_that_did_not_land_leaves_the_pr_url_in_the_error() {
    let message = label_failure(
        "https://github.com/o/r/pull/7",
        7,
        &argv(&["agent-rule-enforcement"]),
        "gh said Not Found (HTTP 404)",
    );
    assert!(
        message.contains("https://github.com/o/r/pull/7"),
        "{message}"
    );
    assert!(message.contains("`agent-rule-enforcement`"), "{message}");
    assert!(
        message.contains("moat gh pr edit 7 --add-label"),
        "{message}"
    );
    assert!(message.contains("moat gh label create"), "{message}");
}

#[test]
fn a_reviewer_github_lists_back_has_landed() {
    let body = br#"{"number":7,"requested_reviewers":[{"login":"AOrwall"}],
        "requested_teams":[{"slug":"backend"}]}"#;
    assert!(
        reviewers_missing_from(body, &argv(&["aorwall", "soaplabs/Backend"])).is_empty(),
        "a login matches requested_reviewers and an org/team its slug in requested_teams"
    );
}

#[test]
fn a_reviewer_absent_from_the_answer_is_reported_missing() {
    let body = br#"{"requested_reviewers":[{"login":"a"}],"requested_teams":[]}"#;
    assert_eq!(
        reviewers_missing_from(body, &argv(&["a", "b", "org/team"])),
        argv(&["b", "org/team"])
    );
    assert_eq!(
        reviewers_missing_from(b"not json", &argv(&["a"])),
        argv(&["a"])
    );
}

#[test]
fn a_reviewer_that_was_not_requested_leaves_the_pr_url_in_the_error() {
    let message = reviewer_failure(
        "https://github.com/o/r/pull/7",
        7,
        &argv(&["octocat"]),
        "gh said Unprocessable Entity (HTTP 422)",
    );
    assert!(
        message.contains("https://github.com/o/r/pull/7"),
        "{message}"
    );
    assert!(message.contains("`octocat`"), "{message}");
    assert!(
        message.contains("moat gh pr edit 7 --add-reviewer"),
        "{message}"
    );
}
