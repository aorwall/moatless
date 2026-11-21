//! Which threads a viewer may dispatch into at all.

use super::fixtures::*;
use super::*;

#[tokio::test]
async fn a_thread_the_viewer_cannot_be_shown_is_refused() {
    let (result, calls) = run_against(turn(None), None).await;
    assert!(matches!(result, Err(CommandRefusal::NotWritable { .. })));
    assert!(calls.is_empty());
}

/// Reading a conversation is not running a turn on it. The two refusals are the
/// same on the wire, deliberately: which one it was says who owns the Task.
#[tokio::test]
async fn a_thread_the_viewer_reads_but_does_not_control_is_refused() {
    let found = conversation(AgentProfile::ClaudeCode);
    let mut dao = MockConversationDao::new();
    dao.expect_readable_by_id()
        .returning(move |_, _| Ok(Some(found.clone())));
    dao.expect_writes_conversation().returning(|_, _| Ok(false));
    dao.expect_interactive_call()
        .returning(|_, _| Ok(Some(asked(mapping::QUESTION_TOOL))));
    let (result, calls) = dispatch_against(dao, turn(None)).await;
    assert!(matches!(result, Err(CommandRefusal::NotWritable { .. })));
    assert!(calls.is_empty());
}

/// A turn into an archived conversation reaches the sink, whose send reopens
/// the Task.
#[tokio::test]
async fn a_turn_into_an_archived_thread_reaches_the_sink() {
    let archived = Conversation {
        closed_at: Some(Utc::now()),
        ..conversation(AgentProfile::ClaudeCode)
    };
    let (result, calls) = run_against(turn(None), Some(archived)).await;
    assert!(result.is_ok(), "{result:?}");
    assert!(
        matches!(calls.as_slice(), [Call::SendTurn { .. }]),
        "{calls:?}"
    );
}

/// Only a turn reopens: any other command on an archived conversation is
/// still refused.
#[tokio::test]
async fn a_rename_of_an_archived_thread_is_refused() {
    let archived = Conversation {
        closed_at: Some(Utc::now()),
        ..conversation(AgentProfile::ClaudeCode)
    };
    let command = json!({
        "type": "thread.meta.update",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "title": "Renamed"
    });
    let (result, calls) = run_against(command, Some(archived)).await;
    assert!(matches!(result, Err(CommandRefusal::NotWritable { .. })));
    assert!(calls.is_empty());
}
