//! Linking a pull request to a thread, and unlinking it.

use super::fixtures::*;
use super::*;

/// The key a link or an unlink names.
fn pull_request_command(kind: &str) -> Value {
    json!({
        "type": format!("thread.pull-request.{kind}"),
        "commandId": "cmd-1",
        "threadId": "task-1",
        "host": "github.com",
        "repository": "acme/web",
        "number": 42,
        "url": "https://github.com/acme/web/pull/42",
        "source": "manual",
    })
}

#[tokio::test]
async fn a_link_and_an_unlink_reach_the_sink_as_the_same_key() {
    for (kind, linked) in [("link", true), ("unlink", false)] {
        let (result, calls) = run(pull_request_command(kind)).await;
        assert!(result.is_ok(), "{kind}");
        assert_eq!(
            calls,
            vec![Call::SetPullRequestLink {
                task_id: "task-1".to_string(),
                host: "github.com".to_string(),
                repository: "acme/web".to_string(),
                number: 42,
                linked,
            }],
            "{kind}"
        );
    }
}

/// A key the server cannot resolve to one pull request is refused before
/// anything writes a Binding, because a link changes what every surface reports
/// for the Thread.
#[tokio::test]
async fn a_link_naming_no_pull_request_is_rejected_before_any_write() {
    for absent in ["host", "repository", "number"] {
        let mut command = pull_request_command("link");
        command[absent] = Value::Null;
        let (result, calls) = run(command).await;
        assert!(
            matches!(result, Err(CommandRefusal::Rejected { .. })),
            "{absent}"
        );
        assert!(calls.is_empty(), "{absent}");
    }
}

/// Gated on control rather than on read: a reader may open a Thread they cannot
/// write, and linking a pull request is a write.
#[tokio::test]
async fn a_link_into_a_thread_the_viewer_does_not_control_is_refused() {
    let found = conversation(AgentType::CLAUDE_CODE);
    let mut dao = MockConversationDao::new();
    dao.expect_readable_by_id()
        .returning(move |_, _| Ok(Some(found.clone())));
    dao.expect_controls_conversation()
        .returning(|_, _| Ok(false));
    let (result, calls) = dispatch_against(dao, pull_request_command("link")).await;
    assert!(matches!(result, Err(CommandRefusal::NotWritable { .. })));
    assert!(calls.is_empty());
}
