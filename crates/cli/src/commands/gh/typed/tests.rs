//! Which invocations `moat gh` types, which fall through to real `gh`, and
//! where a comment body comes from.

use super::*;

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(ToString::to_string).collect()
}

#[test]
fn parses_pr_create_with_typed_flags() {
    let parsed = parse_typed(&argv(&[
        "pr", "create", "--title", "fix: x", "--body", "b", "--head", "feat/x", "--base", "main",
        "--draft", "-R", "o/r", "--task", "t_1",
    ]));
    let Some(TypedInvocation::PrCreate(a)) = parsed else {
        panic!("expected typed pr create, got {parsed:?}");
    };
    assert_eq!(a.title, "fix: x");
    assert_eq!(a.body.as_deref(), Some("b"));
    assert_eq!(a.head.as_deref(), Some("feat/x"));
    assert_eq!(a.base.as_deref(), Some("main"));
    assert!(a.draft);
    assert_eq!(a.repo.as_deref(), Some("o/r"));
    assert_eq!(a.task.as_deref(), Some("t_1"));
}

#[test]
fn parses_pr_create_short_flags() {
    let parsed = parse_typed(&argv(&["pr", "create", "-t", "title", "-b", "body"]));
    assert!(matches!(parsed, Some(TypedInvocation::PrCreate(_))));
}

#[test]
fn pr_create_with_unknown_flag_falls_through() {
    // `--fill` is not typed — must go to the real gh.
    assert_eq!(parse_typed(&argv(&["pr", "create", "--fill"])), None);
    // Missing required --title also falls through.
    assert_eq!(parse_typed(&argv(&["pr", "create"])), None);
}

/// `--body-file` must not demote `pr create` to plain `gh`: plain `gh` opens
/// the PR and returns its URL while binding nothing, so the PR is never
/// connected to the task even though comments posted in the same run are.
#[test]
fn pr_create_with_body_file_stays_on_the_typed_path() {
    let parsed = parse_typed(&argv(&[
        "pr",
        "create",
        "-t",
        "t",
        "--body-file",
        "/tmp/b.md",
        "--head",
        "feat/x",
        "--base",
        "main",
    ]));
    let Some(TypedInvocation::PrCreate(a)) = parsed else {
        panic!("expected typed pr create, got {parsed:?}");
    };
    assert_eq!(
        a.body_file.as_deref(),
        Some(std::path::Path::new("/tmp/b.md"))
    );
    assert!(a.body.is_none());
}

/// `-F` is what real `gh` spells `--body-file`, so an agent copying a gh
/// invocation lands on the typed path too.
#[test]
fn pr_create_accepts_the_gh_short_body_file_flag() {
    let parsed = parse_typed(&argv(&["pr", "create", "-t", "t", "-F", "/tmp/b.md"]));
    assert!(matches!(parsed, Some(TypedInvocation::PrCreate(_))));
}

/// Both spellings at once is a user error, not a demotion — but it still falls
/// through, so real gh reports it rather than moat guessing.
#[test]
fn pr_create_rejects_body_and_body_file_together() {
    assert_eq!(
        parse_typed(&argv(&[
            "pr",
            "create",
            "-t",
            "t",
            "-b",
            "inline",
            "--body-file",
            "/tmp/b.md"
        ])),
        None
    );
}

/// A comment body may come from a file as well; the same demotion cost applied,
/// and the same flag caused it.
#[test]
fn comment_accepts_body_file() {
    let parsed = parse_typed(&argv(&["pr", "comment", "42", "--body-file", "/tmp/b.md"]));
    let Some(TypedInvocation::Comment(a)) = parsed else {
        panic!("expected typed comment, got {parsed:?}");
    };
    assert_eq!(a.number, 42);
    assert!(a.body.is_none());
    assert_eq!(
        a.body_file.as_deref(),
        Some(std::path::Path::new("/tmp/b.md"))
    );
}

/// A comment still needs a body from somewhere.
#[test]
fn comment_without_any_body_falls_through() {
    assert_eq!(parse_typed(&argv(&["pr", "comment", "42"])), None);
}

#[test]
fn body_file_is_read_when_no_inline_body_is_given() {
    let dir = std::env::temp_dir().join("moat-gh-body-file-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("body.md");
    std::fs::write(&path, "from a file").unwrap();

    assert_eq!(
        read_body(None, Some(path.as_path())).unwrap().as_deref(),
        Some("from a file")
    );
    // An inline body wins; clap has already ruled out both being present.
    assert_eq!(
        read_body(Some("inline".into()), Some(path.as_path()))
            .unwrap()
            .as_deref(),
        Some("inline")
    );
    std::fs::remove_file(&path).ok();
}

/// A missing file names the path rather than failing as a bare io error — the
/// agent needs to know which file it meant.
#[test]
fn a_missing_body_file_names_the_path() {
    let err = read_body(None, Some(std::path::Path::new("/nonexistent/body.md")))
        .expect_err("a missing body file is an error");
    assert!(
        err.to_string().contains("/nonexistent/body.md"),
        "error should name the file, got: {err}"
    );
}

/// The demotion warning exists so a fall-through is never silent again.
/// `--help` is not a demotion: the user asked for text.
#[test]
fn help_is_not_reported_as_a_demotion() {
    assert_eq!(parse_typed(&argv(&["pr", "create", "--help"])), None);
}

#[test]
fn parses_pr_and_issue_comment() {
    let parsed = parse_typed(&argv(&["pr", "comment", "42", "--body", "hi"]));
    let Some(TypedInvocation::Comment(a)) = parsed else {
        panic!("expected typed comment, got {parsed:?}");
    };
    assert_eq!(a.number, 42);
    assert_eq!(a.body.as_deref(), Some("hi"));

    assert!(matches!(
        parse_typed(&argv(&["issue", "comment", "7", "-b", "hi", "-R", "o/r"])),
        Some(TypedInvocation::Comment(_))
    ));
}

#[test]
fn comment_with_non_numeric_target_falls_through() {
    // gh accepts URLs/branches as the comment target — we only type numbers.
    assert_eq!(
        parse_typed(&argv(&[
            "pr",
            "comment",
            "https://github.com/o/r/pull/42",
            "--body",
            "hi"
        ])),
        None
    );
    // No --body → editor-based flow → passthrough.
    assert_eq!(parse_typed(&argv(&["pr", "comment", "42"])), None);
}

#[test]
fn parses_pr_reply() {
    let parsed = parse_typed(&argv(&[
        "pr", "reply", "42", "777", "--body", "fixed", "-R", "o/r", "--task", "t_1",
    ]));
    let Some(TypedInvocation::PrReply(a)) = parsed else {
        panic!("expected typed pr reply, got {parsed:?}");
    };
    assert_eq!(a.pr_number, 42);
    assert_eq!(a.comment_id, 777);
    assert_eq!(a.body.as_deref(), Some("fixed"));
    assert_eq!(a.repo.as_deref(), Some("o/r"));
    assert_eq!(a.task.as_deref(), Some("t_1"));
}

#[test]
fn pr_reply_missing_body_falls_through() {
    // No --body → fall through to the real gh (which errors: `pr reply`
    // is moat-only). Better than a half-typed editor flow.
    assert_eq!(parse_typed(&argv(&["pr", "reply", "42", "777"])), None);
}

#[test]
fn pr_reply_non_numeric_positionals_fall_through() {
    assert_eq!(
        parse_typed(&argv(&["pr", "reply", "abc", "777", "--body", "x"])),
        None
    );
    assert_eq!(
        parse_typed(&argv(&["pr", "reply", "42", "not-an-id", "--body", "x"])),
        None
    );
    // Missing comment_id entirely also falls through.
    assert_eq!(
        parse_typed(&argv(&["pr", "reply", "42", "--body", "x"])),
        None
    );
}

#[test]
fn parses_stack_invocation_and_strips_task_flag() {
    let parsed = parse_typed(&argv(&[
        "stack", "link", "207", "208", "--task", "t_1", "--open",
    ]));
    let Some(TypedInvocation::Stack(a)) = parsed else {
        panic!("expected stack invocation, got {parsed:?}");
    };
    assert_eq!(a.task.as_deref(), Some("t_1"));
    assert_eq!(a.args, argv(&["stack", "link", "207", "208", "--open"]));

    let parsed = parse_typed(&argv(&["stack", "submit", "--task=t_1", "--auto"]));
    let Some(TypedInvocation::Stack(a)) = parsed else {
        panic!("expected stack invocation, got {parsed:?}");
    };
    assert_eq!(a.task.as_deref(), Some("t_1"));
    assert_eq!(a.args, argv(&["stack", "submit", "--auto"]));
}

#[test]
fn stack_missing_task_value_falls_through() {
    assert_eq!(parse_typed(&argv(&["stack", "link", "1", "--task"])), None);
    assert_eq!(parse_typed(&argv(&["stack", "link", "1", "--task="])), None);
}

#[test]
fn other_subcommands_fall_through() {
    assert_eq!(parse_typed(&argv(&["pr", "view", "42"])), None);
    assert_eq!(parse_typed(&argv(&["api", "/user"])), None);
    assert_eq!(parse_typed(&argv(&["run", "watch", "1"])), None);
    assert_eq!(parse_typed(&argv(&["issue", "create", "-t", "x"])), None);
    assert_eq!(parse_typed(&argv(&[])), None);
    assert_eq!(parse_typed(&argv(&["pr"])), None);
}

// =======================================================================
// `pr create --label`
// =======================================================================

/// `--label` used to be unknown to the parser, which demoted the call to
/// plain `gh` — losing the task binding, and failing outright when the
/// label did not exist.
#[test]
fn pr_create_takes_labels_on_the_typed_path() {
    let parsed = parse_typed(&argv(&[
        "pr", "create", "--title", "t", "--body", "b", "--label", "one", "-l", "two",
    ]));
    let Some(TypedInvocation::PrCreate(a)) = parsed else {
        panic!("expected typed pr create, got {parsed:?}");
    };
    assert_eq!(a.labels, vec!["one".to_string(), "two".to_string()]);
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
