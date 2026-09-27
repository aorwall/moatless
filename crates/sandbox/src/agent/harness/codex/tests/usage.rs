//! The Turn terminal, and the usage the meter reports with it.

use super::fixtures::*;
use super::*;

/// The [`RunTotals`] the terminal carries, or panics if none was emitted.
fn run_totals(events: &[ProcessEvent]) -> RunTotals {
    events
        .iter()
        .find_map(|e| match e {
            ProcessEvent::RunTotals(t) => Some(t.clone()),
            _ => None,
        })
        .expect("run totals")
}

#[test]
fn turn_completed_emits_usage_result_then_terminated() {
    let mut p = open_process();
    p.absorb(turn_started("turn-1"));
    p.absorb(token_usage(12, 3, 4, 7));
    p.absorb(turn_completed("completed", None));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s.len(), 1, "turn usage carried on a single result block");
    assert_eq!(s[0].block_json()["type"], "turn_result");
    let totals = run_totals(&events);
    assert_eq!(totals.input_tokens, 9, "net of the cached-prefix reads");
    assert_eq!(totals.cache_read_input_tokens, 3);
    assert_eq!(totals.output_tokens, 4);
    assert_eq!(totals.reasoning_output_tokens, 7);
    assert!(matches!(
        events.last(),
        Some(ProcessEvent::Terminated {
            termination: ProcessTermination::Completed
        })
    ));
}

/// The meter measures what occupies the window, which is the last request's own
/// input and output. `total` is the whole Session's spend and keeps rising past
/// the window it would be measured against.
#[test]
fn the_meter_reads_the_last_request_not_the_sessions_spend() {
    let mut p = open_process();
    p.absorb(turn_started("turn-1"));
    p.absorb(token_usage_in_window(
        (30_000, 500),
        (400_000, 9_000),
        200_000,
    ));

    let window = drain(&mut p)
        .into_iter()
        .find_map(|event| match event {
            ProcessEvent::ContextWindow(window) => Some(window),
            _ => None,
        })
        .expect("a reading");
    assert_eq!(window.used_tokens, 30_500);
    assert_eq!(window.max_tokens, Some(200_000));
}

/// A used figure past the window cannot be right. Full is the honest reading of
/// it; a percentage over 100 is not.
#[test]
fn a_reading_past_the_window_reads_as_full() {
    let mut p = open_process();
    p.absorb(turn_started("turn-1"));
    p.absorb(token_usage_in_window(
        (250_000, 1_000),
        (250_000, 1_000),
        200_000,
    ));

    let window = drain(&mut p)
        .into_iter()
        .find_map(|event| match event {
            ProcessEvent::ContextWindow(window) => Some(window),
            _ => None,
        })
        .expect("a reading");
    assert_eq!(window.used_tokens, 200_000);
    assert_eq!(window.max_tokens, Some(200_000));
}

/// `total` is cumulative over the Session and is replayed on resume, so the
/// Turn's own spend is its growth — not the last reading, which would bill the
/// resumed Turn for the whole conversation.
#[test]
fn usage_on_a_resumed_thread_reports_only_what_this_turn_spent() {
    let mut p = open_process();
    // Replayed before the Turn opens: what the Session had already spent.
    p.absorb(token_usage(1000, 100, 50, 10));
    p.absorb(turn_started("turn-2"));
    // Two model requests inside the Turn, each a fresh cumulative reading.
    p.absorb(token_usage(1600, 150, 80, 20));
    p.absorb(token_usage(2200, 200, 110, 30));
    p.absorb(turn_completed("completed", None));

    let events = drain(&mut p);
    let totals = run_totals(&events);
    assert_eq!(totals.input_tokens, 1100, "net of the cached-prefix reads");
    assert_eq!(totals.cache_read_input_tokens, 100);
    assert_eq!(totals.output_tokens, 60);
    assert_eq!(totals.reasoning_output_tokens, 20);
}

#[test]
fn a_turn_with_no_usage_reported_still_emits_a_result_block() {
    let mut p = open_process();
    p.absorb(turn_completed("completed", None));
    let events = drain(&mut p);
    let s = sources(&events);
    assert_eq!(s.len(), 1);
    assert_eq!(run_totals(&events).input_tokens, 0);
}

/// An interrupted Turn is a normal end, not a failure: the driver owns
/// cancelled-ness and maps the termination once it knows it asked for it.
#[test]
fn an_interrupted_turn_terminates_as_completed_not_failed() {
    let mut p = open_process();
    p.absorb(turn_completed("interrupted", None));
    let events = drain(&mut p);
    assert!(
        matches!(
            events.last(),
            Some(ProcessEvent::Terminated {
                termination: ProcessTermination::Completed
            })
        ),
        "got {events:?}"
    );
}

#[test]
fn a_failed_turn_with_no_named_cause_still_terminates_as_a_failure() {
    let mut p = open_process();
    p.absorb(turn_completed(
        "failed",
        Some(json!({ "message": "something went wrong" })),
    ));
    let events = drain(&mut p);
    match events.last() {
        Some(ProcessEvent::Terminated {
            termination: ProcessTermination::Failed(error),
        }) => {
            assert_eq!(error.class, ErrorClass::InvalidInput);
            assert_eq!(error.session_impact, SessionImpact::Intact);
        }
        other => panic!("expected a failed terminal, got {other:?}"),
    }
}

/// The `error` notification is not a terminal: Codex sends it while it is still
/// retrying, and `turn/completed` is the authority.
#[test]
fn an_error_notification_does_not_end_the_turn() {
    let mut p = open_process();
    p.absorb(ServerEvent::Error(parse(json!({
        "error": { "message": "stream disconnected" },
        "willRetry": true,
    }))));
    assert!(drain(&mut p).is_empty());
}
