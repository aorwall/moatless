//! Stopping, settling, snoozing, following and re-scoping a thread.

use super::fixtures::*;
use super::*;

/// The Stop button under a running turn sends the interrupt, and it must reach
/// the agent-only stop: a person ending a turn keeps their Sandbox. A session
/// stop is the one that releases it.
#[tokio::test]
async fn interrupt_stops_the_agent_and_session_stop_stops_the_sandbox() {
    for (tag, expected) in [
        (
            "thread.turn.interrupt",
            Call::Interrupt {
                task_id: "task-1".to_string(),
            },
        ),
        (
            "thread.session.stop",
            Call::Stop {
                task_id: "task-1".to_string(),
            },
        ),
    ] {
        let command = json!({
            "type": tag,
            "commandId": "cmd-1",
            "threadId": "task-1",
            "createdAt": "2026-07-01T09:00:00.000Z"
        });
        let (result, calls) = run(command).await;
        assert!(result.is_ok(), "{tag}");
        assert_eq!(calls, vec![expected], "{tag}");
    }
}

#[tokio::test]
async fn settle_commands_reach_the_sink() {
    for (command, expected) in [
        (
            json!({
                "type": "thread.settle",
                "commandId": "cmd-1",
                "threadId": "task-1",
            }),
            Call::Settle {
                task_id: "task-1".to_string(),
            },
        ),
        (
            json!({
                "type": "thread.unsettle",
                "commandId": "cmd-1",
                "threadId": "task-1",
            }),
            Call::Unsettle {
                task_id: "task-1".to_string(),
            },
        ),
        (
            json!({
                "type": "thread.unsnooze",
                "commandId": "cmd-1",
                "threadId": "task-1",
            }),
            Call::Unsnooze {
                task_id: "task-1".to_string(),
            },
        ),
    ] {
        let tag = command["type"].as_str().unwrap().to_string();
        let (result, calls) = run(command).await;
        assert!(result.is_ok(), "{tag}");
        assert_eq!(calls, vec![expected], "{tag}");
    }
}

#[tokio::test]
async fn snooze_reaches_the_sink_with_its_wake_time() {
    let command = json!({
        "type": "thread.snooze",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "snoozedUntil": "2099-07-01T09:00:00.000Z",
    });
    let (result, calls) = run(command).await;
    assert!(result.is_ok());
    assert_eq!(
        calls,
        vec![Call::Snooze {
            task_id: "task-1".to_string(),
            snoozed_until: "2099-07-01T09:00:00+00:00".to_string(),
        }]
    );
}

#[tokio::test]
async fn snooze_rejects_a_missing_wake_time_before_any_write() {
    let command = json!({
        "type": "thread.snooze",
        "commandId": "cmd-1",
        "threadId": "task-1",
    });
    let (result, calls) = run(command).await;
    assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
    assert!(calls.is_empty());
}

#[tokio::test]
async fn snooze_rejects_an_unparseable_wake_time_before_any_write() {
    let command = json!({
        "type": "thread.snooze",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "snoozedUntil": "tomorrow",
    });
    let (result, calls) = run(command).await;
    assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
    assert!(calls.is_empty());
}

#[tokio::test]
async fn snooze_rejects_a_past_wake_time_before_any_write() {
    let command = json!({
        "type": "thread.snooze",
        "commandId": "cmd-1",
        "threadId": "task-1",
        "snoozedUntil": "2020-07-01T09:00:00.000Z",
    });
    let (result, calls) = run(command).await;
    assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
    assert!(calls.is_empty());
}

#[tokio::test]
async fn following_and_unfollowing_reach_the_sink() {
    for (tag, expected) in [
        (
            "thread.follow",
            Call::Follow {
                task_id: "task-1".to_string(),
            },
        ),
        (
            "thread.unfollow",
            Call::Unfollow {
                task_id: "task-1".to_string(),
            },
        ),
    ] {
        let command = json!({
            "type": tag,
            "commandId": "cmd-1",
            "threadId": "task-1",
        });
        let (result, calls) = run(command).await;
        assert!(result.is_ok(), "{tag}");
        assert_eq!(calls, vec![expected], "{tag}");
    }
}

#[tokio::test]
async fn a_visibility_change_reaches_the_sink_typed() {
    for value in ["public", "private"] {
        let command = json!({
            "type": "thread.visibility.set",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "visibility": value,
        });
        let (result, calls) = run(command).await;
        assert!(result.is_ok(), "{value}");
        assert_eq!(
            calls,
            vec![Call::SetVisibility {
                task_id: "task-1".to_string(),
                visibility: Visibility::try_from(value).unwrap(),
            }],
            "{value}"
        );
    }
}

#[tokio::test]
async fn a_visibility_change_rejects_an_unknown_level_before_any_write() {
    for command in [
        json!({
            "type": "thread.visibility.set",
            "commandId": "cmd-1",
            "threadId": "task-1",
        }),
        json!({
            "type": "thread.visibility.set",
            "commandId": "cmd-1",
            "threadId": "task-1",
            "visibility": "secret",
        }),
    ] {
        let (result, calls) = run(command).await;
        assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
        assert!(calls.is_empty());
    }
}

#[tokio::test]
async fn pin_commands_reach_the_sink_with_their_order_keys() {
    for (command, expected) in [
        (
            json!({ "type": "thread.pin", "threadId": "task-1", "orderKey": "a0" }),
            Call::Pin {
                task_id: "task-1".to_string(),
                order_key: Some("a0".to_string()),
            },
        ),
        (
            json!({ "type": "thread.pin", "threadId": "task-1" }),
            Call::Pin {
                task_id: "task-1".to_string(),
                order_key: None,
            },
        ),
        (
            json!({ "type": "thread.unpin", "threadId": "task-1" }),
            Call::Unpin {
                task_id: "task-1".to_string(),
            },
        ),
        (
            json!({ "type": "thread.pin.reorder", "threadId": "task-1", "orderKey": "a1" }),
            Call::SetPinOrderKey {
                task_id: "task-1".to_string(),
                order_key: "a1".to_string(),
            },
        ),
        (
            json!({ "type": "thread.active.reorder", "threadId": "task-1", "orderKey": "Zz" }),
            Call::SetActiveOrderKey {
                task_id: "task-1".to_string(),
                order_key: "Zz".to_string(),
            },
        ),
    ] {
        let tag = command["type"].as_str().unwrap().to_string();
        let (result, calls) = run(command).await;
        assert!(result.is_ok(), "{tag}");
        assert_eq!(calls, vec![expected], "{tag}");
    }
}

#[tokio::test]
async fn auto_settle_set_reaches_the_sink_and_needs_enabled() {
    for enabled in [false, true] {
        let (result, calls) = run(json!({
            "type": "thread.auto-settle.set",
            "threadId": "task-1",
            "enabled": enabled,
        }))
        .await;
        assert!(result.is_ok(), "{enabled}");
        assert_eq!(
            calls,
            vec![Call::SetAutoSettle {
                task_id: "task-1".to_string(),
                enabled,
            }]
        );
    }
    let (result, calls) =
        run(json!({ "type": "thread.auto-settle.set", "threadId": "task-1" })).await;
    assert!(matches!(result, Err(CommandRefusal::Rejected { .. })));
    assert!(calls.is_empty());
}

/// The listing decodes an order key as a non-empty string, so a blank one
/// stored here would fail the viewer's whole sidebar.
#[tokio::test]
async fn a_reorder_without_a_usable_key_is_rejected_before_any_write() {
    for command in [
        json!({ "type": "thread.pin.reorder", "threadId": "task-1" }),
        json!({ "type": "thread.active.reorder", "threadId": "task-1", "orderKey": "  " }),
        json!({ "type": "thread.pin", "threadId": "task-1", "orderKey": "" }),
    ] {
        let (result, calls) = run(command.clone()).await;
        assert!(
            matches!(result, Err(CommandRefusal::Rejected { .. })),
            "{command}"
        );
        assert!(calls.is_empty(), "{command}");
    }
}
