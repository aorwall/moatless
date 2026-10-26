//! The conversation row the shell lists: its archive, settlement, visibility,
//! owner and liveness fields.

use super::fixtures::*;
use super::*;

/// `archivedAt` is the Task's close time.
///
/// An open conversation carries null, which puts it in the sidebar rather than
/// the archive; a closed one carries the time, which T3 reads to tell the two
/// apart.
#[test]
fn a_closed_conversation_is_archived_at_the_time_it_was_closed() {
    let open = one_thread(&running_conversation("task-a", "ws-a"));
    assert_eq!(open["archivedAt"], Value::Null);

    let closed_at = DateTime::parse_from_rfc3339("2026-07-04T09:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc);
    let mut row = running_conversation("task-a", "ws-a");
    row.closed_at = Some(closed_at);
    assert_eq!(
        one_thread(&row)["archivedAt"],
        json!("2026-07-04T09:00:00.000Z")
    );
}

#[test]
fn settlement_and_snooze_state_are_null_when_unset() {
    let thread = one_thread(&running_conversation("task-a", "ws-a"));
    for key in ["settledOverride", "settledAt", "snoozedUntil", "snoozedAt"] {
        assert_eq!(thread[key], Value::Null, "{key}");
    }
}

#[test]
fn shell_and_detail_rows_carry_the_same_settlement_and_snooze_state() {
    let settled_at = DateTime::parse_from_rfc3339("2026-07-02T09:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc);
    let snoozed_until = DateTime::parse_from_rfc3339("2026-07-03T09:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc);
    let snoozed_at = DateTime::parse_from_rfc3339("2026-07-02T10:00:00.000Z")
        .unwrap()
        .with_timezone(&Utc);
    let conversation = Conversation {
        settled_override: Some("settled".to_string()),
        settled_at: Some(settled_at),
        snoozed_until: Some(snoozed_until),
        snoozed_at: Some(snoozed_at),
        last_read_at: None,
        ..running_conversation("task-a", "ws-a")
    };

    let shell = one_thread(&conversation);
    let detail = thread_detail(
        &conversation,
        "ws-a",
        &AgentModelDefaults::default(),
        &transcript(Vec::new(), Vec::new()),
        &OriginContext {
            frontend_url: "http://localhost:3000",
            source_task_names: &HashMap::new(),
        },
    );
    for key in ["settledOverride", "settledAt", "snoozedUntil", "snoozedAt"] {
        assert_eq!(shell[key], detail[key], "{key}");
    }
    assert_eq!(shell["settledOverride"], json!("settled"));
    assert_eq!(shell["settledAt"], json!("2026-07-02T09:00:00.000Z"));
    assert_eq!(shell["snoozedUntil"], json!("2026-07-03T09:00:00.000Z"));
    assert_eq!(shell["snoozedAt"], json!("2026-07-02T10:00:00.000Z"));
}

#[test]
fn shell_and_detail_rows_carry_the_task_visibility() {
    let conversation = Conversation {
        visibility: moatless_types::task::Visibility::Public,
        ..running_conversation("task-a", "ws-a")
    };
    let detail = thread_detail(
        &conversation,
        "ws-a",
        &AgentModelDefaults::default(),
        &transcript(Vec::new(), Vec::new()),
        &OriginContext {
            frontend_url: "http://localhost:3000",
            source_task_names: &HashMap::new(),
        },
    );
    assert_eq!(one_thread(&conversation)["visibility"], json!("public"));
    assert_eq!(detail["visibility"], json!("public"));
    assert_eq!(
        one_thread(&running_conversation("task-b", "ws-a"))["visibility"],
        json!("private")
    );
}

#[test]
fn a_row_names_its_owner_only_when_the_listing_read_one() {
    // The listing leaves the name out for a Task the viewer owns, so a row
    // that carries one is a row that belongs to somebody else. Null and not
    // absent: the client reads the key to clear the previous answer.
    assert_eq!(
        one_thread(&conversation("task-a", "ws-a"))["ownerName"],
        Value::Null
    );
    assert_eq!(
        one_thread(&running_conversation("task-b", "ws-a"))["ownerName"],
        json!("Ada Lovelace")
    );
}

/// Both tools park the run on a person, and the client reads one boolean for
/// both: its sidebar status, its snooze rule and its awareness phase all key on
/// `hasPendingUserInput`. `hasActionableProposedPlan` stays the narrower answer,
/// naming which of the two is waiting.
#[test]
fn a_parked_plan_reads_as_pending_user_input_the_way_a_parked_question_does() {
    let mut row = running_conversation("task-a", "ws-a");
    assert_eq!(one_thread(&row)["hasPendingUserInput"], false);

    row.pending_plan = true;
    assert_eq!(one_thread(&row)["hasPendingUserInput"], true);
    assert_eq!(one_thread(&row)["hasActionableProposedPlan"], true);

    row.pending_plan = false;
    row.pending_question = true;
    assert_eq!(one_thread(&row)["hasPendingUserInput"], true);
    assert_eq!(one_thread(&row)["hasActionableProposedPlan"], false);
}

#[test]
fn a_running_moat_cmd_command_makes_the_row_read_as_working() {
    let thread = one_thread(&Conversation {
        running_commands: 1,
        ..conversation("task-a", "ws-a")
    });

    assert_eq!(
        thread["backgroundLiveness"],
        Value::String("working".into())
    );
}

#[test]
fn a_row_with_no_running_command_reports_no_background_liveness() {
    // Null rather than absent: the sidebar reads the key to decide the row
    // is resting, and a Task that just finished its last command has to say
    // so rather than leave the client on its previous answer.
    let thread = one_thread(&conversation("task-a", "ws-a"));

    assert_eq!(thread["backgroundLiveness"], Value::Null);
}

#[test]
fn background_liveness_reports_the_axis_not_the_count() {
    // The contract has two literals and neither is a number: a second
    // command must not change what the row says.
    let one = one_thread(&Conversation {
        running_commands: 1,
        ..conversation("task-a", "ws-a")
    });
    let many = one_thread(&Conversation {
        running_commands: 7,
        ..conversation("task-a", "ws-a")
    });

    assert_eq!(one["backgroundLiveness"], many["backgroundLiveness"]);
}

/// The instance id is a branded slug, and an id that does not match is refused
/// at decode rather than downgraded.
#[test]
fn the_provider_instance_is_a_well_formed_slug() {
    for agent_profile in ["claude-code", "codex", "claude-code-fable", "made-up"] {
        let mut row = running_conversation("task-a", "ws-a");
        row.agent_profile = agent_profile.to_string().into();
        let instance = one_thread(&row)["modelSelection"]["instanceId"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(instance.len() <= 64, "{instance} is too long for the slug");
        let mut chars = instance.chars();
        assert!(chars.next().is_some_and(|c| c.is_ascii_alphabetic()));
        assert!(chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }
}

#[test]
fn a_conversation_that_never_ran_reports_no_turn_and_no_session() {
    let thread = one_thread(&conversation("task-a", "ws-a"));
    assert_eq!(thread["latestTurn"], Value::Null);
    assert_eq!(thread["session"], Value::Null);
    assert_eq!(thread["worktreePath"], Value::Null);
    assert_eq!(thread["latestUserMessageAt"], Value::Null);
}

#[test]
fn a_blank_conversation_name_falls_back_to_new_thread() {
    let mut row = conversation("task-a", "ws-a");
    row.name = Some("   ".to_string());
    assert_eq!(one_thread(&row)["title"], "New thread");
    row.name = None;
    assert_eq!(one_thread(&row)["title"], "New thread");
}

/// Two Moatless states mean *not finished* and both land on `running`; the
/// shared word `interrupted` does not mean the same thing on the two sides.
#[test]
fn every_moatless_turn_state_has_a_counterpart() {
    assert_eq!(turn_state("active"), Some("running"));
    assert_eq!(turn_state("interrupted"), Some("running"));
    assert_eq!(turn_state("stopped"), Some("interrupted"));
    assert_eq!(turn_state("completed"), Some("completed"));
    assert_eq!(turn_state("error"), Some("error"));
    assert_eq!(turn_state("from-a-newer-backend"), None);
}

#[test]
fn an_unmappable_turn_state_reports_no_turn_rather_than_a_guess() {
    let mut row = running_conversation("task-a", "ws-a");
    row.turn_status = Some("from-a-newer-backend".to_string());
    let thread = one_thread(&row);
    assert_eq!(thread["latestTurn"], Value::Null);
    assert_eq!(thread["session"]["activeTurnId"], Value::Null);
}

/// The session's turn and the row's turn are the same turn, so the two ids have
/// to agree — the client uses one to look up the other.
#[test]
fn an_open_turn_is_the_sessions_active_turn() {
    let thread = one_thread(&running_conversation("task-a", "ws-a"));
    assert_eq!(thread["latestTurn"]["turnId"], "task-a:3");
    assert_eq!(thread["session"]["activeTurnId"], "task-a:3");
    assert_eq!(thread["session"]["status"], "running");
}

#[test]
fn a_finished_turn_leaves_the_session_with_no_active_turn() {
    let mut row = running_conversation("task-a", "ws-a");
    row.turn_status = Some("completed".to_string());
    row.turn_ended_at = Some(Utc::now());
    let thread = one_thread(&row);
    assert_eq!(thread["latestTurn"]["state"], "completed");
    assert!(thread["latestTurn"]["completedAt"].is_string());
    assert_eq!(thread["session"]["activeTurnId"], Value::Null);
    assert_eq!(thread["session"]["status"], "ready");
}

/// A turn opens from inside the Sandbox, so between a person sending and the
/// agent starting there is a window with work in flight and no turn to show for
/// it.
///
/// The session reads as running across it, not as a thread with nothing
/// happening.
#[test]
fn a_sandbox_coming_up_for_a_message_is_a_session_working() {
    let mut row = running_conversation("task-a", "ws-a");
    // Sent, and nothing has run yet: the Sandbox is still being provisioned.
    row.turn_number = None;
    row.turn_status = None;
    row.turn_started_at = None;
    row.sandbox_started_at = None;
    assert_eq!(one_thread(&row)["session"]["status"], "running");
    assert_eq!(
        one_thread(&row)["session"]["activeTurnId"],
        Value::Null,
        "no turn has opened yet, and the session says so"
    );

    // Answered: the newest turn ended after the message, so nothing is owed.
    row.turn_number = Some(3);
    row.turn_status = Some("completed".to_string());
    row.turn_ended_at = Some(Utc::now());
    row.sandbox_started_at = Some(Utc::now());
    assert_eq!(one_thread(&row)["session"]["status"], "ready");

    // Asked again, on a Sandbox that has to come up first.
    row.latest_queued_message_at = Some(Utc::now());
    assert_eq!(one_thread(&row)["session"]["status"], "running");
}

/// A sandbox wake owes a turn as much as a person's message does, but it is
/// not the person writing.
#[test]
fn a_queued_wake_after_a_settled_turn_reads_as_running() {
    let mut row = running_conversation("task-a", "ws-a");
    let asked = Utc::now() - chrono::Duration::minutes(5);
    row.latest_user_message_at = Some(asked);
    row.turn_status = Some("completed".to_string());
    row.turn_ended_at = Some(asked + chrono::Duration::minutes(1));
    row.latest_queued_message_at = Some(Utc::now());

    let thread = one_thread(&row);
    assert_eq!(thread["session"]["status"], "running");
    assert_eq!(thread["latestUserMessageAt"], nullable_instant(Some(asked)));
}

/// A turn that finished without recording when says nothing either way, and is
/// read as settled rather than leaving the session running for good.
#[test]
fn a_turn_that_ended_at_no_recorded_time_owes_nothing() {
    let mut row = running_conversation("task-a", "ws-a");
    row.turn_status = Some("completed".to_string());
    row.turn_ended_at = None;
    assert_eq!(one_thread(&row)["session"]["status"], "ready");
}

#[test]
fn sandbox_intent_becomes_the_sessions_status() {
    let mut row = running_conversation("task-a", "ws-a");
    row.turn_status = Some("completed".to_string());

    row.sandbox_started_at = None;
    assert_eq!(one_thread(&row)["session"]["status"], "starting");

    row.sandbox_started_at = Some(Utc::now());
    row.sandbox_desired_state_raw = Some(SandboxDesiredState::Stopped.as_str().to_string());
    assert_eq!(one_thread(&row)["session"]["status"], "stopped");

    row.sandbox_stop_reason_raw = Some(SandboxStopReason::Error.as_str().to_string());
    assert_eq!(one_thread(&row)["session"]["status"], "error");

    row.sandbox_desired_state_raw = Some(SandboxDesiredState::Removed.as_str().to_string());
    assert_eq!(
        one_thread(&row)["worktreePath"],
        Value::Null,
        "a removed sandbox has had its working copy reclaimed"
    );
}

/// The error a usage limit ends a turn with carries its reset time, and the
/// client's error banner reads it from here.
#[test]
fn the_newest_turns_error_is_the_sessions_last_error() {
    let mut row = running_conversation("task-a", "ws-a");
    assert_eq!(one_thread(&row)["session"]["lastError"], Value::Null);

    row.turn_status = Some("error".to_string());
    row.turn_error =
        Some("You've hit your usage limit. (usage limit resets at 2026-10-03T18:05:00Z)".into());
    assert_eq!(
        one_thread(&row)["session"]["lastError"],
        "You've hit your usage limit. (usage limit resets at 2026-10-03T18:05:00Z)"
    );
}

#[test]
fn pin_and_order_keys_are_null_when_unset() {
    let thread = one_thread(&running_conversation("task-a", "ws-a"));
    for key in [
        "pinnedAt",
        "pinOrderKey",
        "activeOrderKey",
        "autoSettleDisabledAt",
    ] {
        assert_eq!(thread[key], Value::Null, "{key}");
    }
}

#[test]
fn a_row_carries_the_viewers_pin_and_order_keys() {
    let conversation = Conversation {
        pinned_at: Some(instant_at("2026-07-02T09:00:00.000Z")),
        pin_order_key: Some("a0".to_string()),
        active_order_key: Some("Zz".to_string()),
        auto_settle_disabled_at: Some(instant_at("2026-07-03T09:00:00.000Z")),
        ..running_conversation("task-a", "ws-a")
    };
    let thread = one_thread(&conversation);
    assert_eq!(thread["pinnedAt"], json!("2026-07-02T09:00:00.000Z"));
    assert_eq!(
        thread["autoSettleDisabledAt"],
        json!("2026-07-03T09:00:00.000Z")
    );
    assert_eq!(thread["pinOrderKey"], json!("a0"));
    assert_eq!(thread["activeOrderKey"], json!("Zz"));
}

/// The contract types a key as a trimmed non-empty string, and one that fails
/// that fails the decode of the viewer's whole listing.
#[test]
fn a_blank_order_key_is_reported_as_none() {
    let conversation = Conversation {
        pin_order_key: Some("  ".to_string()),
        ..running_conversation("task-a", "ws-a")
    };
    assert_eq!(one_thread(&conversation)["pinOrderKey"], Value::Null);
}
