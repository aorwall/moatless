//! Session announcement, and the thread attribution that keeps a native
//! sub-agent out of this Turn.

use super::fixtures::*;
use super::*;

#[test]
fn thread_started_yields_session_ready_then_input_accepted() {
    let mut p = process();
    p.input_id = Some(InputId("in-1".to_string()));
    p.absorb(thread_started("thread-A"));
    let events = drain(&mut p);
    assert!(matches!(
        events[0],
        ProcessEvent::SessionReady { ref session, .. } if session.0 == "thread-A"
    ));
    assert!(matches!(
        events[1],
        ProcessEvent::InputAccepted { ref input } if input.0 == "in-1"
    ));
    assert_eq!(events.len(), 2);
}

/// A resume emits no `thread/started`, so the Session is announced from the
/// request response instead. When both happen — a fresh thread does notify —
/// the second announcement must not duplicate the pair.
#[test]
fn the_session_is_announced_exactly_once() {
    let mut p = process();
    p.input_id = Some(InputId("in-1".to_string()));
    p.announce_session("thread-A".to_string());
    p.absorb(thread_started("thread-A"));
    let events = drain(&mut p);
    assert_eq!(events.len(), 2, "got {events:?}");
}

// ── Native sub-agents: one connection, several threads ───────────

/// A native sub-agent runs on its own thread over the same connection.
///
/// Its `turn/completed` must never be taken as this Turn's terminal — that
/// would end the run mid-work and report the sub-agent's last message as the
/// answer.
#[test]
fn a_sub_agents_turn_completion_does_not_end_this_turn() {
    let mut p = open_process();
    p.absorb(turn_started_on("thread-A", "turn-1"));
    p.absorb(item_completed_on(
        "thread-A",
        agent_message("item-1", "mine"),
    ));
    // The sub-agent's whole life, interleaved with the parent still running.
    p.absorb(turn_started_on("thread-B", "turn-2"));
    p.absorb(item_completed_on(
        "thread-B",
        agent_message("item-2", "theirs"),
    ));
    p.absorb(token_usage_on("thread-B", 90_000, 400));
    p.absorb(turn_completed_on("thread-B", "completed"));

    let events = drain(&mut p);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ProcessEvent::Terminated { .. })),
        "the sub-agent ended this Turn: {events:?}"
    );
    let texts: Vec<String> = sources(&events)
        .iter()
        .map(|s| s.block_json()["text"].as_str().unwrap_or("-").to_string())
        .collect();
    assert_eq!(texts, vec!["mine".to_string()], "got {texts:?}");

    // This Turn's own terminal still ends it.
    p.absorb(turn_completed_on("thread-A", "completed"));
    assert!(matches!(
        drain(&mut p).last(),
        Some(ProcessEvent::Terminated {
            termination: ProcessTermination::Completed
        })
    ));
}

/// A sub-agent's spend is its own. Billing this Turn for it made the reported
/// context window jump between two threads' totals on every reading.
#[test]
fn a_sub_agents_usage_is_not_billed_to_this_turn() {
    let mut p = open_process();
    p.absorb(token_usage_on("thread-A", 1_000, 50));
    p.absorb(turn_started_on("thread-A", "turn-1"));
    p.absorb(token_usage_on("thread-B", 90_000, 4_000));
    p.absorb(token_usage_on("thread-A", 1_600, 80));
    p.absorb(turn_completed_on("thread-A", "completed"));

    let events = drain(&mut p);
    let usage = sources(&events)
        .iter()
        .find(|s| s.block_json()["type"] == "turn_result")
        .and_then(|s| s.usage.clone())
        .expect("usage");
    assert_eq!(usage["input_tokens"], 600);
    assert_eq!(usage["output_tokens"], 30);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ProcessEvent::ContextWindow(w) if w.used_tokens == 94_000)),
        "the sub-agent's reading was published as this Turn's: {events:?}"
    );
}

/// The steerability gate and `turn/interrupt` both address the Turn by id, and
/// the server rejects a stale one. A sub-agent's `turn/started` must not become
/// what this process remembers.
#[test]
fn a_sub_agents_turn_does_not_replace_the_id_this_turn_is_steered_by() {
    let mut p = open_process();
    p.absorb(turn_started_on("thread-A", "turn-1"));
    p.absorb(turn_started_on("thread-B", "turn-2"));
    assert_eq!(p.turn_id.as_deref(), Some("turn-1"));
    assert_eq!(p.thread_id.as_deref(), Some("thread-A"));
}

/// A server that attributes a notification to no thread gets the benefit of the
/// doubt. Dropping an unattributed `turn/completed` would leave the Turn with
/// no terminal at all, which costs more than the mis-attribution it prevents.
#[test]
fn a_notification_naming_no_thread_is_taken_as_this_turns() {
    let mut p = open_process();
    p.absorb(turn_completed("completed", None));
    assert!(matches!(
        drain(&mut p).last(),
        Some(ProcessEvent::Terminated {
            termination: ProcessTermination::Completed
        })
    ));
}
