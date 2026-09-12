//! Answering what a thread is waiting on: a question, a plan, a multi-select.

use super::fixtures::*;
use super::*;

#[tokio::test]
async fn an_answer_reaches_the_agent_as_a_tool_response() {
    let command = json!({
        "type": "thread.user-input.respond",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "requestId": "toolu_1",
        "answers": { "choice": "yes" },
        "createdAt": "2026-07-01T09:00:00.000Z"
    });
    let (result, calls) = run(command).await;
    assert!(result.is_ok());
    assert_eq!(
        calls,
        vec![Call::AnswerQuestion {
            task_id: "task-1".to_string(),
            tool_use_id: "toolu_1".to_string(),
            tool_name: mapping::QUESTION_TOOL.to_string(),
            payload: Some(json!({ "answers": { "choice": "yes" } })),
        }]
    );
}

/// A plan is answered through the question joined to it, so the same
/// command has to become a verdict rather than a blob of answers. Only the
/// approve option approves; anything else is a rejection carrying what was
/// said, which is what makes "No, use the second approach" reach the agent
/// as a rejection *and* as the reason.
#[tokio::test]
async fn a_plan_is_answered_as_a_verdict() {
    for (said, expected) in [
        (mapping::PLAN_APPROVE, json!({ "approved": true })),
        (
            "Reject: use the second approach",
            json!({ "approved": false, "feedback": "Reject: use the second approach" }),
        ),
    ] {
        let (result, calls) = run_answering(
            answer(json!({ mapping::PLAN_QUESTION: said })),
            Some(asked(mapping::PLAN_TOOL)),
        )
        .await;
        assert!(result.is_ok(), "{said}");
        assert_eq!(
            calls,
            vec![Call::AnswerQuestion {
                task_id: "task-1".to_string(),
                tool_use_id: "toolu_1".to_string(),
                tool_name: mapping::PLAN_TOOL.to_string(),
                payload: Some(expected),
            }],
            "{said}"
        );
    }
}

/// A multi-select answer arrives as a list and the CLI merges the value into
/// the tool input unread, so it is joined here rather than sent as an array
/// the agent would have to parse.
#[tokio::test]
async fn a_multi_select_answer_reaches_the_agent_as_one_string() {
    let (_, calls) = run_answering(
        answer(json!({ "Which?": ["Postgres", "Redis"] })),
        Some(asked(mapping::QUESTION_TOOL)),
    )
    .await;
    let Some(Call::AnswerQuestion { payload, .. }) = calls.first() else {
        panic!("the answer reaches the sink: {calls:?}");
    };
    assert_eq!(
        payload.as_ref().unwrap()["answers"]["Which?"],
        "Postgres, Redis"
    );
}

/// An answer naming a call this thread never stored is not a failed send —
/// there is nothing to send it to. The two read differently to the person
/// who clicked.
#[tokio::test]
async fn an_answer_to_a_call_that_was_never_stored_is_refused() {
    let (result, calls) = run_answering(answer(json!({ "q": "yes" })), None).await;
    assert!(
        matches!(result, Err(CommandRefusal::Rejected { .. })),
        "{result:?}"
    );
    assert!(calls.is_empty());
}

/// The call came back already — from the Moatless UI, or from a second
/// person. Sending a second answer would reach an agent that has moved on.
#[tokio::test]
async fn an_answer_to_a_settled_call_is_refused() {
    let (result, calls) = run_answering(
        answer(json!({ "q": "yes" })),
        Some(InteractiveCall {
            tool_name: mapping::QUESTION_TOOL.to_string(),
            settled: true,
        }),
    )
    .await;
    assert!(
        matches!(result, Err(CommandRefusal::Rejected { .. })),
        "{result:?}"
    );
    assert!(calls.is_empty());
}
