//! The `modelSwitch` a user turn carries when its model or effort differs
//! from the turn before.

use super::*;

/// One turn's two Messages: the person's, asking for `effort`, and the
/// agent's reply, reporting `model` on `harness`.
fn turn_on(turn: i32, harness: &str, model: &str, effort: Option<&str>) -> Vec<TranscriptMessage> {
    let mut asked = message(
        &format!("user-{turn}"),
        Some(turn),
        "Go on",
        MessageType::User,
    );
    asked.effort = effort.map(str::to_string);
    let mut replied = message(
        &format!("agent-{turn}"),
        Some(turn),
        "Done",
        MessageType::Assistant,
    );
    replied.harness = Some(harness.to_string());
    replied.model = Some(model.to_string());
    vec![asked, replied]
}

fn user_items(transcript: &Transcript) -> Vec<Value> {
    let conversation = open_turn(conversation("task-1"), 4, "completed");
    let names = HashMap::new();
    let (items, _) = turn_items_and_messages(&conversation, transcript, &origins(&names));
    items
        .into_iter()
        .filter(|item| item["type"] == "user_message")
        .collect()
}

#[test]
fn a_turn_on_another_model_or_effort_carries_the_switch() {
    let transcript = Transcript {
        messages: [
            turn_on(1, "claude-code", "claude-opus-5-5", Some("high")),
            turn_on(2, "claude-code", "claude-opus-5-5", None),
            turn_on(3, "claude-code", "claude-opus-5-5", Some("low")),
            turn_on(4, "codex", "gpt-5.6-sol", None),
        ]
        .concat(),
        ..Transcript::default()
    };
    let items = user_items(&transcript);
    assert!(items[0].get("modelSwitch").is_none());
    // No effort asked for: the turn keeps the one before, rather than
    // reading as a switch to the default.
    assert!(items[1].get("modelSwitch").is_none());
    assert_eq!(
        items[2]["modelSwitch"],
        json!({
            "from": { "instanceId": "claudeAgent", "model": "claude-opus-5-5",
                      "options": [{ "id": "effort", "value": "high" }] },
            "to": { "instanceId": "claudeAgent", "model": "claude-opus-5-5",
                    "options": [{ "id": "effort", "value": "low" }] },
        })
    );
    assert_eq!(
        items[3]["modelSwitch"]["to"],
        json!({ "instanceId": "codex", "model": "gpt-5.6-sol" })
    );
}

/// A turn no agent reply has reported on yet has no model to compare, so it
/// carries no switch until one arrives.
#[test]
fn a_turn_without_a_reply_carries_no_switch() {
    let mut messages = turn_on(1, "claude-code", "claude-opus-5-5", None);
    messages.push(message("user-2", Some(2), "And this", MessageType::User));
    let transcript = Transcript {
        messages,
        ..Transcript::default()
    };
    assert!(
        user_items(&transcript)
            .iter()
            .all(|item| item.get("modelSwitch").is_none())
    );
}
