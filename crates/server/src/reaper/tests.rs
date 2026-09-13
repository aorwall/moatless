//! Unit tests for the reaper's pure decision functions: what the cycle groups,
//! what it stops, closes and reclaims, and which turn it reconciles.

use super::*;

/// The reconcile event's shape. The id must be deterministic per `(task, turn)`
/// so a cycle that repeats while the terminal event is still missing upserts
/// one message row instead of stacking duplicates.
#[test]
fn reconcile_error_event_is_deterministic_per_task_and_turn() {
    let turn = UnclosedTurn {
        task_id: "task-9".to_string(),
        turn_number: Some(4),
        agent_type: Some("claude".to_string()),
    };

    let event = reconcile_error_event(&turn);
    assert_eq!(event.event_id, "reconcile-task-9-4");
    assert_eq!(event.event_type, EventType::Agent);
    assert_eq!(event.sub_type, "error");
    assert_eq!(event.task_id, "task-9");
    assert!(event.is_error);
    assert_eq!(event.agent_type.as_deref(), Some("claude"));
    assert_eq!(event.turn_number, Some(4));
    assert_eq!(
        event.payload.get("reason").and_then(|v| v.as_str()),
        Some("reconciled_orphaned_turn")
    );

    assert_eq!(reconcile_error_event(&turn).event_id, event.event_id);
}

/// A turn with no number still gets a stable id rather than colliding with
/// another numberless turn on the same task.
#[test]
fn reconcile_error_event_labels_a_numberless_turn() {
    let turn = UnclosedTurn {
        task_id: "task-3".to_string(),
        turn_number: None,
        agent_type: None,
    };
    assert_eq!(
        reconcile_error_event(&turn).event_id,
        "reconcile-task-3-unknown"
    );
}

/// A parked run is doing no work, so it does not hold its sandbox open past the
/// ordinary TTL. Everything the agent is actually running does.
#[test]
fn a_waiting_agent_does_not_hold_its_sandbox() {
    assert!(!agent_holds_the_sandbox("waiting"));
    assert!(!agent_holds_the_sandbox("idle"));
    assert!(!agent_holds_the_sandbox("stopped"));
    assert!(agent_holds_the_sandbox("running"));
    // A status this build does not know is treated as live.
    assert!(agent_holds_the_sandbox("from-a-newer-sandbox"));
}

fn members(ids: &[&str]) -> Vec<String> {
    ids.iter().map(|id| (*id).to_string()).collect()
}

/// A fork runs inside its parent's pod, so both Tasks reach the passes that
/// decide whether anything is still driving a turn.
#[test]
fn a_fork_is_grouped_under_the_sandbox_it_runs_in() {
    let sandboxes = group_members(
        &members(&["sandbox-a", "sandbox-b"]),
        vec![
            ("sandbox-a".to_string(), "sandbox-a".to_string()),
            ("fork-1".to_string(), "sandbox-a".to_string()),
            ("closed-task".to_string(), "sandbox-c".to_string()),
        ],
    );

    assert_eq!(sandboxes[0].members, members(&["sandbox-a", "fork-1"]));
    // A Sandbox whose owner row is not open still decides on its own id.
    assert_eq!(sandboxes[1].members, members(&["sandbox-b"]));
    assert_eq!(
        live_task_ids(&sandboxes),
        members(&["sandbox-a", "fork-1", "sandbox-b"])
    );
}

/// The defect this pass fixes: the owner went idle, the fork was working, and
/// the pod both of them live in was stopped anyway.
#[test]
fn a_working_member_keeps_the_whole_sandbox() {
    let now = Utc::now();
    let long_ago = now - chrono::Duration::hours(3);
    let lifecycle = HashMap::from([
        ("sandbox-a".to_string(), ("stopped".to_string(), long_ago)),
        ("fork-1".to_string(), ("started".to_string(), long_ago)),
    ]);
    let latest = HashMap::from([
        ("sandbox-a".to_string(), long_ago),
        ("fork-1".to_string(), long_ago),
    ]);

    let floor = fold_reap_floor(&members(&["sandbox-a", "fork-1"]), &lifecycle, &latest, now);

    assert_eq!(floor, Some(now));
    assert_eq!(decide_reap(30, floor, now), ReapDecision::Keep);
}

/// Every Member idle past the folded TTL is what a stop needs.
#[test]
fn a_sandbox_whose_every_member_is_idle_is_stopped() {
    let now = Utc::now();
    let lifecycle = HashMap::new();
    let latest = HashMap::from([
        ("sandbox-a".to_string(), now - chrono::Duration::hours(3)),
        ("fork-1".to_string(), now - chrono::Duration::hours(2)),
    ]);

    let floor = fold_reap_floor(&members(&["sandbox-a", "fork-1"]), &lifecycle, &latest, now);

    assert_eq!(floor, Some(now - chrono::Duration::hours(2)));
    assert_eq!(decide_reap(30, floor, now), ReapDecision::Stop);
}

/// `0` is indefinite, so a pinned Member pins the pod it shares.
#[test]
fn one_pinned_member_pins_the_sandbox() {
    let ttl_map = HashMap::from([("fork-1".to_string(), 0)]);
    assert_eq!(
        fold_ttl_minutes(&members(&["sandbox-a", "fork-1"]), &ttl_map, 30),
        0
    );
}

/// Otherwise the most patient Member sets the TTL: a shorter one must not take
/// the pod out from under it.
#[test]
fn the_largest_member_ttl_wins() {
    let ttl_map = HashMap::from([("sandbox-a".to_string(), 10)]);
    assert_eq!(
        fold_ttl_minutes(&members(&["sandbox-a", "fork-1"]), &ttl_map, 45),
        45
    );
}

/// A parked turn is not finished — it resumes where it left off, so the
/// reconciler leaves it open even if the sandbox is reclaimed.
///
/// Covers a lost `taskTurn.active`: the agent works under a new run while the
/// row still names the previous one.
#[test]
fn a_live_run_the_row_never_covered_opens_a_turn() {
    assert_eq!(
        classify_live_run("running", Some("run-2"), Some("run-1")),
        LiveRunVerdict::OpenTurn("run-2")
    );
}

/// A parked run is still a run — the same reading `agent_turn_is_over` already
/// applies on the orphaned-turn side.
#[test]
fn a_waiting_run_still_opens_a_turn() {
    assert_eq!(
        classify_live_run("waiting", Some("run-2"), Some("run-1")),
        LiveRunVerdict::OpenTurn("run-2")
    );
}

#[test]
fn a_turn_row_from_before_run_attribution_yields_to_a_named_run() {
    assert_eq!(
        classify_live_run("running", Some("run-2"), None),
        LiveRunVerdict::OpenTurn("run-2")
    );
}

/// Both records describe one run and disagree about whether it ended. Writing
/// either way would bury a coordinator defect.
#[test]
fn the_same_run_on_both_sides_is_a_contradiction_not_a_repair() {
    assert_eq!(
        classify_live_run("running", Some("run-1"), Some("run-1")),
        LiveRunVerdict::Contradiction
    );
}

/// An older sandbox image sends no `run_id`, so nothing can be attributed.
/// Opening a turn on that would double-count every long run.
#[test]
fn an_unattributable_heartbeat_never_opens_a_turn() {
    assert_eq!(
        classify_live_run("running", None, Some("run-1")),
        LiveRunVerdict::Contradiction
    );
    assert_eq!(
        classify_live_run("running", None, None),
        LiveRunVerdict::Contradiction
    );
}

#[test]
fn a_finished_agent_is_left_alone_whatever_the_run_ids_say() {
    for status in ["idle", "stopped"] {
        assert_eq!(
            classify_live_run(status, Some("run-2"), Some("run-1")),
            LiveRunVerdict::Ignore,
            "{status}"
        );
    }
}

#[test]
fn a_waiting_agents_turn_is_not_over() {
    assert!(!agent_turn_is_over("waiting"));
    assert!(agent_turn_is_over("idle"));
    assert!(agent_turn_is_over("stopped"));
    assert!(!agent_turn_is_over("running"));
}

fn ts(seconds_ago: i64) -> DateTime<Utc> {
    Utc::now() - chrono::Duration::seconds(seconds_ago)
}

// =========================================================================
// decide_reap
// =========================================================================

#[test]
fn ttl_zero_is_indefinite() {
    let now = Utc::now();
    let very_old = now - chrono::Duration::days(7);
    let decision = decide_reap(0, Some(very_old), now);
    assert_eq!(
        decision,
        ReapDecision::Skip("ttl_minutes is 0 (indefinite)")
    );
}

#[test]
fn negative_ttl_is_treated_as_indefinite() {
    let now = Utc::now();
    let very_old = now - chrono::Duration::days(7);
    let decision = decide_reap(-1, Some(very_old), now);
    assert_eq!(
        decision,
        ReapDecision::Skip("ttl_minutes is 0 (indefinite)")
    );
}

#[test]
fn missing_activity_skips() {
    let now = Utc::now();
    let decision = decide_reap(60, None, now);
    assert_eq!(decision, ReapDecision::Skip("no activity timestamp"));
}

#[test]
fn within_ttl_keeps() {
    let now = Utc::now();
    let decision = decide_reap(60, Some(ts(30 * 60)), now);
    assert_eq!(decision, ReapDecision::Keep);
}

#[test]
fn beyond_ttl_stops() {
    let now = Utc::now();
    let decision = decide_reap(60, Some(ts(90 * 60)), now);
    assert_eq!(decision, ReapDecision::Stop);
}

#[test]
fn pinned_12h_keeps_after_2h_idle() {
    let now = Utc::now();
    let decision = decide_reap(720, Some(ts(2 * 60 * 60)), now);
    assert_eq!(decision, ReapDecision::Keep);
}

#[test]
fn pinned_12h_stops_after_13h_idle() {
    let now = Utc::now();
    let decision = decide_reap(720, Some(ts(13 * 60 * 60)), now);
    assert_eq!(decision, ReapDecision::Stop);
}

// =========================================================================
// decide_close
// =========================================================================

#[test]
fn close_zero_inactive_ttl_is_indefinite() {
    let now = Utc::now();
    let very_old = now - chrono::Duration::days(30);
    let decision = decide_close(0, Some(very_old), now);
    assert_eq!(
        decision,
        CloseDecision::Skip("inactive_ttl_days is 0 (indefinite)")
    );
}

#[test]
fn close_beyond_ttl_closes() {
    let now = Utc::now();
    // 7-day TTL, last activity 8 days ago → close.
    let decision = decide_close(7, Some(now - chrono::Duration::days(8)), now);
    assert_eq!(decision, CloseDecision::Close);
}

#[test]
fn close_within_ttl_keeps() {
    let now = Utc::now();
    // 7-day TTL, last activity 1 day ago → keep.
    let decision = decide_close(7, Some(now - chrono::Duration::days(1)), now);
    assert_eq!(decision, CloseDecision::Keep);
}

// =========================================================================
// decide_cold
// =========================================================================

#[test]
fn cold_zero_hours_never_reclaims() {
    let now = Utc::now();
    assert!(!decide_cold(0, now - chrono::Duration::days(30), now));
}

#[test]
fn cold_beyond_the_clock_reclaims() {
    let now = Utc::now();
    assert!(decide_cold(24, now - chrono::Duration::hours(25), now));
}

#[test]
fn cold_within_the_clock_keeps_the_volume() {
    let now = Utc::now();
    assert!(!decide_cold(24, now - chrono::Duration::hours(23), now));
}

#[test]
fn cold_clock_runs_from_the_stop_not_the_activity() {
    // A task stopped 2h ago after a month of chat is warm; the branch reads
    // the stop, so a long-idle-but-recently-stopped task keeps its volume.
    let now = Utc::now();
    assert!(!decide_cold(24, now - chrono::Duration::hours(2), now));
}

// =========================================================================
// is_terminal_agent_sub_type / compute_reap_floor (Fix E)
// =========================================================================

#[test]
fn started_is_not_terminal() {
    assert!(!is_terminal_agent_sub_type("started"));
}

#[test]
fn completed_stopped_error_are_terminal() {
    assert!(is_terminal_agent_sub_type("completed"));
    assert!(is_terminal_agent_sub_type("stopped"));
    assert!(is_terminal_agent_sub_type("error"));
}

#[test]
fn running_agent_pins_floor_to_now() {
    // Latest agent event is `started` ⇒ agent running ⇒ floor is `now`,
    // regardless of how stale the message/activity floor is.
    let now = Utc::now();
    let stale = now - chrono::Duration::hours(5);
    let event = ("started".to_string(), now - chrono::Duration::hours(3));
    let floor = compute_reap_floor(Some(&event), Some(stale), now);
    assert_eq!(floor, Some(now));
}

#[test]
fn running_agent_is_never_reaped_even_far_past_ttl() {
    // End-to-end through decide_reap: a running agent with a very stale
    // activity floor and a short TTL still Keeps.
    let now = Utc::now();
    let stale = now - chrono::Duration::hours(10);
    let event = ("started".to_string(), now - chrono::Duration::hours(9));
    let floor = compute_reap_floor(Some(&event), Some(stale), now);
    assert_eq!(decide_reap(60, floor, now), ReapDecision::Keep);
}

#[test]
fn stopped_agent_floor_dates_from_stop_time() {
    // Terminal latest event, stop more recent than the message floor ⇒
    // floor = stop time, so the TTL counts from agent-stop.
    let now = Utc::now();
    let old_message = now - chrono::Duration::hours(5);
    let stopped_at = now - chrono::Duration::minutes(30);
    let event = ("completed".to_string(), stopped_at);
    let floor = compute_reap_floor(Some(&event), Some(old_message), now);
    assert_eq!(floor, Some(stopped_at));
    // 60-min TTL, stopped 30 min ago → still within TTL → Keep.
    assert_eq!(decide_reap(60, floor, now), ReapDecision::Keep);
}

#[test]
fn stopped_agent_reaped_after_ttl_from_stop() {
    // Terminal latest event, stopped longer ago than the TTL ⇒ Stop.
    let now = Utc::now();
    let stopped_at = now - chrono::Duration::minutes(90);
    let event = ("stopped".to_string(), stopped_at);
    let floor = compute_reap_floor(Some(&event), None, now);
    assert_eq!(floor, Some(stopped_at));
    assert_eq!(decide_reap(60, floor, now), ReapDecision::Stop);
}

#[test]
fn stopped_agent_uses_newer_activity_floor() {
    // A user message AFTER the agent stopped keeps the floor at the message
    // time (max of the two), so post-stop activity still counts.
    let now = Utc::now();
    let stopped_at = now - chrono::Duration::hours(2);
    let newer_message = now - chrono::Duration::minutes(10);
    let event = ("error".to_string(), stopped_at);
    let floor = compute_reap_floor(Some(&event), Some(newer_message), now);
    assert_eq!(floor, Some(newer_message));
}

#[test]
fn no_agent_event_falls_back_to_activity_floor() {
    let now = Utc::now();
    let activity = now - chrono::Duration::minutes(45);
    let floor = compute_reap_floor(None, Some(activity), now);
    assert_eq!(floor, Some(activity));

    // None activity floor stays None (decide_reap then Skips).
    let floor_none = compute_reap_floor(None, None, now);
    assert_eq!(floor_none, None);
    assert_eq!(
        decide_reap(60, floor_none, now),
        ReapDecision::Skip("no activity timestamp")
    );
}

#[test]
fn orphan_deleted_task_is_gc_immediately() {
    assert_eq!(decide_orphan(false), OrphanDecision::Delete);
}

#[test]
fn orphan_open_task_is_never_gc() {
    assert_eq!(decide_orphan(true), OrphanDecision::Keep);
}

#[test]
fn existing_closed_task_is_left_to_lifecycle() {
    assert_eq!(decide_orphan(true), OrphanDecision::Keep);
}
