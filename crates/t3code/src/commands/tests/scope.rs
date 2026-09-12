//! Which threads a viewer may dispatch into at all.

use super::fixtures::*;
use super::*;

#[tokio::test]
async fn a_thread_the_viewer_cannot_be_shown_is_refused() {
    let (result, calls) = run_against(turn(None), None).await;
    assert!(matches!(result, Err(CommandRefusal::NotWritable { .. })));
    assert!(calls.is_empty());
}

/// Reading a conversation is not running a turn on it. The two refusals are
/// the same on the wire, deliberately: which one it was says who owns the
/// Task.
#[tokio::test]
async fn a_thread_the_viewer_reads_but_does_not_control_is_refused() {
    let found = conversation(AgentType::CLAUDE_CODE);
    let mut dao = MockConversationDao::new();
    dao.expect_readable_by_id()
        .returning(move |_, _| Ok(Some(found.clone())));
    dao.expect_controls_conversation()
        .returning(|_, _| Ok(false));
    dao.expect_interactive_call()
        .returning(|_, _| Ok(Some(asked(mapping::QUESTION_TOOL))));
    let (result, calls) = dispatch_against(dao, turn(None)).await;
    assert!(matches!(result, Err(CommandRefusal::NotWritable { .. })));
    assert!(calls.is_empty());
}

/// An archived conversation resolves — the viewer follows it and may read
/// it, which is what lets them open it — and a turn into it is still
/// refused. Unarchiving is the way back, and it does not come through here.
#[tokio::test]
async fn a_turn_into_an_archived_thread_is_refused() {
    let archived = Conversation {
        closed_at: Some(Utc::now()),
        ..conversation(AgentType::CLAUDE_CODE)
    };
    let (result, calls) = run_against(turn(None), Some(archived)).await;
    assert!(matches!(result, Err(CommandRefusal::NotWritable { .. })));
    assert!(calls.is_empty());
}
