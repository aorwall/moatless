//! Which invocations `moat gh` types, which fall through to real `gh`, and
//! where a comment body comes from.

use super::*;

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(ToString::to_string).collect()
}

/// The refusal message, so a test can check it names the flag that caused it.
fn refusal(args: &[&str]) -> String {
    match parse_typed(&argv(args)) {
        Some(TypedInvocation::Refused(reason)) => reason,
        other => panic!("expected a refusal for {args:?}, got {other:?}"),
    }
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
fn pr_create_closes_the_task_on_close_only_when_asked() {
    let parse = |extra: &[&str]| {
        let mut args = argv(&["pr", "create", "--title", "t", "--body", "b"]);
        args.extend(argv(extra));
        match parse_typed(&args) {
            Some(TypedInvocation::PrCreate(a)) => a.close_task_on_close,
            other => panic!("expected typed pr create, got {other:?}"),
        }
    };
    assert!(parse(&["--close-task-on-close"]));
    assert!(!parse(&[]));
}

#[test]
fn parses_pr_create_short_flags() {
    let parsed = parse_typed(&argv(&["pr", "create", "-t", "title", "-b", "body"]));
    assert!(matches!(parsed, Some(TypedInvocation::PrCreate(_))));
}

/// Plain `gh` would open the PR unbound and without the `Moatless task:`
/// footer, so a CI hand-back could not find the task (feedback 038c31fb).
#[test]
fn pr_create_with_an_unmodeled_flag_is_refused_not_passed_through() {
    let reason = refusal(&["pr", "create", "--attach", "x.png", "-t", "t", "-b", "b"]);
    assert!(reason.contains("`--attach`"), "names the flag: {reason}");
    assert!(reason.contains("unbound"), "names the cost: {reason}");

    assert!(refusal(&["pr", "create", "--fill"]).contains("`--fill`"));
    assert!(refusal(&["pr", "create"]).contains("--title"));
}

/// `--body-file` is a flag agents reach for, so it must stay typed rather than
/// be refused.
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

#[test]
fn pr_create_rejects_body_and_body_file_together() {
    refusal(&[
        "pr",
        "create",
        "-t",
        "t",
        "-b",
        "inline",
        "--body-file",
        "/tmp/b.md",
    ]);
}

/// A comment body may come from a file as well.
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
fn comment_without_any_body_is_refused() {
    refusal(&["pr", "comment", "42"]);
    refusal(&["issue", "comment", "42"]);
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

/// Real `gh`'s help would advertise flags such as `--attach` that are refused.
#[test]
fn help_on_a_proxied_write_lists_moats_own_flags() {
    let Some(TypedInvocation::Help(text)) = parse_typed(&argv(&["pr", "create", "--help"])) else {
        panic!("expected moat's help");
    };
    assert!(text.contains("--close-task-on-close"));
    assert!(!text.contains("--attach"));
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

/// gh accepts URLs and branches as the comment target; moat types numbers.
#[test]
fn comment_with_non_numeric_target_is_refused() {
    refusal(&[
        "pr",
        "comment",
        "https://github.com/o/r/pull/42",
        "--body",
        "hi",
    ]);
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
fn pr_reply_missing_body_is_refused() {
    refusal(&["pr", "reply", "42", "777"]);
}

#[test]
fn pr_reply_non_numeric_positionals_are_refused() {
    refusal(&["pr", "reply", "abc", "777", "--body", "x"]);
    refusal(&["pr", "reply", "42", "not-an-id", "--body", "x"]);
    refusal(&["pr", "reply", "42", "--body", "x"]);
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

/// An unknown flag refuses the call, so the flags `gh pr create` users reach
/// for stay on the typed path.
#[test]
fn pr_create_takes_labels_and_reviewers_on_the_typed_path() {
    let parsed = parse_typed(&argv(&[
        "pr",
        "create",
        "--title",
        "t",
        "--body",
        "b",
        "--label",
        "one",
        "-l",
        "two",
        "--reviewer",
        "alice,org/team",
        "-r",
        "bob",
    ]));
    let Some(TypedInvocation::PrCreate(a)) = parsed else {
        panic!("expected typed pr create, got {parsed:?}");
    };
    assert_eq!(a.labels, argv(&["one", "two"]));
    assert_eq!(a.reviewers, argv(&["alice", "org/team", "bob"]));
}
