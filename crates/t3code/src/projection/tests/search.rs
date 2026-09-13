//! What a search match reports: which rows are reachable, and the window cut
//! around the matched text.

use super::*;

fn matched(workspace_id: Option<&str>, from_user: bool, text: &str) -> ConversationMatch {
    ConversationMatch {
        task_id: "task-a".into(),
        workspace_id: workspace_id.map(ToString::to_string),
        from_user,
        matched_text: text.into(),
        matched_at: None,
    }
}

/// The client routes on `projectId`. A match without one is a row that cannot
/// be opened, so it is left out rather than shown and then failing.
#[test]
fn a_match_on_a_conversation_with_no_workspace_is_not_reported() {
    let reported = thread_matches(
        &[
            matched(None, true, "outbox"),
            matched(Some("workspace-a"), true, "outbox"),
        ],
        "outbox",
    );

    assert_eq!(reported.len(), 1);
    assert_eq!(reported[0]["projectId"], "workspace-a");
    assert_eq!(reported[0]["source"], "user");
    assert_eq!(reported[0]["messageCreatedAt"], Value::Null);
}

#[test]
fn the_agents_own_words_are_reported_as_the_agents() {
    let reported = thread_matches(&[matched(Some("workspace-a"), false, "outbox")], "outbox");
    assert_eq!(reported[0]["source"], "assistant");
}

/// A snippet is one line; the text it comes from is not.
#[test]
fn a_short_match_keeps_its_text_with_the_line_breaks_collapsed() {
    assert_eq!(
        snippet("the outbox\n  is  full\n", "outbox"),
        "the outbox is full"
    );
}

/// Cutting from the start would drop the match, and the match is the whole
/// reason the row is on screen.
#[test]
fn a_long_match_is_windowed_around_the_match_and_fits_the_contracts_cap() {
    let text = format!("{} NEEDLE {}", "a ".repeat(400), "b ".repeat(400));
    let snippet = snippet(&text, "needle");

    assert!(snippet.contains("NEEDLE"), "the match survives the window");
    assert!(snippet.starts_with('…') && snippet.ends_with('…'));
    assert!(
        snippet.chars().count() <= SNIPPET_MAX,
        "{} characters is over the contract's cap",
        snippet.chars().count()
    );
}

/// The window is measured in characters because the cap is on the decoded
/// string, and a byte cut lands inside a character the moment anything is not
/// ASCII.
#[test]
fn a_window_never_cuts_a_character_in_half() {
    let text = format!("{} nål {}", "ä ".repeat(400), "ö ".repeat(400));
    let snippet = snippet(&text, "nål");

    assert!(snippet.contains("nål"));
    assert!(snippet.chars().count() <= SNIPPET_MAX);
}
