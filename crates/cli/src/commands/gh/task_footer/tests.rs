//! Which `pr edit` argvs carry a body, and how [`kept`] lays the footer back on.

use super::*;

fn strs(args: &[&str]) -> Vec<String> {
    args.iter().map(ToString::to_string).collect()
}

#[test]
fn finds_the_body_in_every_spelling() {
    for (args, want) in [
        (
            strs(&["12", "--body", "new"]),
            BodySource::Inline("new".into()),
        ),
        (strs(&["12", "-b", "new"]), BodySource::Inline("new".into())),
        (
            strs(&["--body=new", "12"]),
            BodySource::Inline("new".into()),
        ),
        (
            strs(&["12", "--body-file", "b.md"]),
            BodySource::File("b.md".into()),
        ),
        (strs(&["-F", "-", "12"]), BodySource::File("-".into())),
    ] {
        let edit = parse(&args);
        assert_eq!(edit.body, Some(want), "for {args:?}");
        assert_eq!(edit.selector.as_deref(), Some("12"), "for {args:?}");
    }
}

#[test]
fn reads_selector_and_repo_past_valued_and_valueless_flags() {
    let edit = parse(&strs(&[
        "--add-label",
        "x",
        "--remove-milestone",
        "-R",
        "o/r",
        "https://github.com/o/r/pull/7",
        "-t",
        "title",
    ]));
    assert_eq!(
        edit.selector.as_deref(),
        Some("https://github.com/o/r/pull/7")
    );
    assert_eq!(edit.repo.as_deref(), Some("o/r"));
    assert_eq!(edit.body, None);
}

#[test]
fn leaves_other_commands_and_bodyless_edits_alone() {
    for args in [
        strs(&["pr", "edit", "12", "--add-label", "x"]),
        strs(&["pr", "view", "12", "--body", "x"]),
        strs(&["issue", "edit", "12", "--body", "x"]),
    ] {
        assert_eq!(
            keep_task_footer(args.clone(), false, None).unwrap().args,
            args
        );
    }
}

#[test]
fn a_body_that_keeps_its_footer_is_passed_on_unread_as_a_file() {
    let body = "new\n\n---\nMoatless task: https://x/tasks/t_1\n";
    let args = strs(&["pr", "edit", "12", "-b", body, "--add-label", "x"]);
    let call = keep_task_footer(args, false, None).unwrap();
    assert_eq!(call.args[..4], strs(&["pr", "edit", "12", "--body-file"]));
    assert_eq!(call.args[5..], strs(&["--add-label", "x"]));
    assert_eq!(std::fs::read_to_string(&call.args[4]).unwrap(), body);
}

#[test]
fn takes_the_last_footer_line() {
    let body = "Moatless task: quoted\n\nbody\n\n---\nMoatless task: https://x/tasks/t_1\n";
    assert_eq!(
        footer_line(body).as_deref(),
        Some("Moatless task: https://x/tasks/t_1")
    );
    assert_eq!(footer_line("no footer here"), None);
}

#[test]
fn lays_the_footer_out_as_pr_create_does() {
    let line = "Moatless task: https://x/tasks/t_1";
    assert_eq!(
        with_footer("new body\n\n", line),
        "new body\n\n---\nMoatless task: https://x/tasks/t_1\n"
    );
    assert_eq!(with_footer("", line), format!("{line}\n"));
}

#[test]
fn kept_appends_the_current_footer_and_reads_only_when_needed() {
    let current = || Ok("old\n\n---\nMoatless task: https://x/tasks/t_1\n".to_string());
    assert_eq!(
        kept("new".into(), current),
        "new\n\n---\nMoatless task: https://x/tasks/t_1\n"
    );
    assert_eq!(kept("new".into(), || Ok("no footer".into())), "new");
    let own = "new\n\n---\nMoatless task: https://x/tasks/t_2\n".to_string();
    assert_eq!(
        kept(own.clone(), || panic!(
            "read a body that has its own footer"
        )),
        own
    );
    assert_eq!(kept("new".into(), || Err("offline".into())), "new");
    let mention = "see the `Moatless task:` footer".to_string();
    assert_eq!(
        kept(mention.clone(), current),
        format!("{mention}\n\n---\nMoatless task: https://x/tasks/t_1\n")
    );
}
