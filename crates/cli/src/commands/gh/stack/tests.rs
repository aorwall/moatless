//! What the gh-stack passthrough binds, and what it leaves alone.

use super::*;

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(ToString::to_string).collect()
}

#[test]
fn empty_task_id_is_absent_for_stack_binding() {
    assert_eq!(non_empty_task_id(None), None);
    assert_eq!(non_empty_task_id(Some(String::new())), None);
    assert_eq!(non_empty_task_id(Some("  ".into())), None);
    assert_eq!(non_empty_task_id(Some("t_1".into())), Some("t_1".into()));
}

#[test]
fn extracts_stack_link_operands_around_flags() {
    assert_eq!(
        stack_link_operands(&argv(&[
            "--base",
            "main",
            "207",
            "--remote=origin",
            "feat/top",
            "--open"
        ])),
        argv(&["207", "feat/top"])
    );
}
