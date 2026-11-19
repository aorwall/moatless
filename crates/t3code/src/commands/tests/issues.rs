//! Linking a Linear issue to a thread, and unlinking it.

use super::fixtures::*;
use super::*;

fn issue_command(kind: &str) -> Value {
    json!({
        "type": format!("thread.issue.{kind}"),
        "commandId": "cmd-1",
        "threadId": "task-1",
        "tracker": "linear",
        "host": "linear.app",
        "scope": "SP",
        "key": "SP-12",
        "projectId": "ws-1",
        "url": "https://linear.app/acme/issue/SP-12",
        "displayKey": "SP-12",
        "title": "Fix the picker",
        "source": "manual",
    })
}

#[tokio::test]
async fn a_link_and_an_unlink_reach_the_sink_as_the_same_key() {
    for (kind, linked) in [("link", true), ("unlink", false)] {
        let (result, calls) = run(issue_command(kind)).await;
        assert!(result.is_ok(), "{kind}");
        assert_eq!(
            calls,
            vec![Call::SetIssueLink {
                task_id: "task-1".to_string(),
                scope: "SP".to_string(),
                key: "SP-12".to_string(),
                linked,
            }],
            "{kind}"
        );
    }
}

#[tokio::test]
async fn an_issue_on_another_tracker_is_rejected_before_any_write() {
    let mut command = issue_command("link");
    command["tracker"] = json!("github");
    let (result, calls) = run(command).await;
    assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
    assert!(calls.is_empty());
}
