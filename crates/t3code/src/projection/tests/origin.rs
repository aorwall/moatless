//! Where a message came from: another Task, an external adapter, or the
//! composer, which carries no origin at all.

use super::fixtures::*;
use super::*;

/// A message typed into the composer carries no origin key at all —
/// byte-identical to a conversation before this field existed.
#[test]
fn a_message_typed_into_the_composer_carries_no_origin() {
    let origin = message_origin(
        &message(1, MessageType::User, "Hi"),
        &OriginContext {
            frontend_url: "https://moatless.example",
            source_task_names: &HashMap::new(),
        },
    );
    assert!(origin.is_none());
}

/// A Message authored by another Task names and links that Task.
#[test]
fn a_message_from_another_task_names_and_links_that_task() {
    let mut msg = message(1, MessageType::User, "Handoff");
    msg.source_task_id = Some("task-xyz".into());
    let names = HashMap::from([("task-xyz".to_string(), "Upstream planning".to_string())]);
    let origin = message_origin(
        &msg,
        // Trailing slash on the base is trimmed, not doubled.
        &OriginContext {
            frontend_url: "https://moatless.example/",
            source_task_names: &names,
        },
    )
    .expect("a task-authored message has an origin");
    assert_eq!(origin["kind"], "task");
    assert_eq!(origin["label"], "Upstream planning");
    assert_eq!(origin["url"], "https://moatless.example/tasks/task-xyz");
    assert_eq!(origin["user"], Value::Null);
}

/// A Task the reader cannot see resolves to no name, and a link they could not
/// follow is worse than none.
#[test]
fn a_task_origin_the_reader_cannot_see_has_neither_name_nor_link() {
    let mut msg = message(1, MessageType::User, "Handoff");
    msg.source_task_id = Some("task-hidden".into());
    let origin = message_origin(
        &msg,
        &OriginContext {
            frontend_url: "https://moatless.example",
            source_task_names: &HashMap::new(),
        },
    )
    .expect("origin present");
    assert_eq!(origin["kind"], "task");
    assert_eq!(origin["label"], Value::Null);
    assert_eq!(origin["url"], Value::Null);
}

/// A task origin wins even where the row also carries adapter columns.
#[test]
fn a_task_origin_wins_over_adapter_columns() {
    let mut msg = message(1, MessageType::User, "Handoff");
    msg.source_task_id = Some("task-xyz".into());
    msg.external_adapter = Some("github_pr".into());
    msg.external_subject_key = Some("octo/repo#42".into());
    let names = HashMap::from([("task-xyz".to_string(), "Upstream".to_string())]);
    let origin = message_origin(
        &msg,
        &OriginContext {
            frontend_url: "https://moatless.example",
            source_task_names: &names,
        },
    )
    .expect("origin present");
    assert_eq!(origin["kind"], "task");
}

/// A GitHub PR message derives its URL from the subject key and event id and
/// shows the handle the external system knows the writer by.
#[test]
fn a_github_pr_message_derives_its_url_and_shows_the_external_author() {
    let mut msg = message(1, MessageType::User, "Please rebase");
    msg.external_adapter = Some("github_pr".into());
    msg.external_subject_key = Some("octo/repo#42".into());
    msg.external_event_id = Some("github:ic:99887766".into());
    msg.external_source_name = Some("octo/repo".into());
    msg.external_author = Some("octocat".into());
    let origin = message_origin(
        &msg,
        &OriginContext {
            frontend_url: "https://moatless.example",
            source_task_names: &HashMap::new(),
        },
    )
    .expect("origin present");
    assert_eq!(origin["kind"], "github_pr");
    assert_eq!(origin["label"], "octo/repo#42");
    assert_eq!(
        origin["url"],
        "https://github.com/octo/repo/pull/42#issuecomment-99887766"
    );
    assert_eq!(origin["user"], "octocat");
}

/// Any other adapter takes its label and URL from the Binding this Task holds,
/// and falls back to the Moatless user when no external handle was captured.
#[test]
fn an_external_message_without_a_captured_handle_falls_back_to_the_moatless_user() {
    let mut msg = message(1, MessageType::User, "Filed from Linear");
    msg.external_adapter = Some("linear".into());
    // What a Linear subscription matches on is its team id; the issue's own
    // identifier is on the Binding.
    msg.external_source_name = Some("f1f0b3a2-team-uuid".into());
    msg.binding_label = Some("SP-2454".into());
    msg.binding_url = Some("https://linear.app/soap-labs/issue/SP-2454".into());
    msg.sent_by_name = Some("Ada Lovelace".into());
    let origin = message_origin(
        &msg,
        &OriginContext {
            frontend_url: "https://moatless.example",
            source_task_names: &HashMap::new(),
        },
    )
    .expect("origin present");
    assert_eq!(origin["kind"], "linear");
    assert_eq!(origin["label"], "SP-2454");
    assert_eq!(origin["url"], "https://linear.app/soap-labs/issue/SP-2454");
    assert_eq!(origin["user"], "Ada Lovelace");
}

/// An adapter whose Binding names no subject keeps whatever matched the
/// subscription rather than going unlabelled.
#[test]
fn an_external_message_whose_binding_names_nothing_falls_back_to_the_source_name() {
    let mut msg = message(1, MessageType::User, "From Slack");
    msg.external_adapter = Some("slack".into());
    msg.external_source_name = Some("#eng".into());
    let origin = message_origin(
        &msg,
        &OriginContext {
            frontend_url: "https://moatless.example",
            source_task_names: &HashMap::new(),
        },
    )
    .expect("origin present");
    assert_eq!(origin["label"], "#eng");
    // Slack keeps channel coordinates on its Binding, not a link.
    assert_eq!(origin["url"], Value::Null);
}

/// The GitHub URL, and the fragment each kind of comment opens at.
#[test]
fn a_github_pr_url_is_built_from_the_subject_key_and_event_id() {
    assert_eq!(
        github_pr_url(Some("octo/repo#42"), Some("github:rc:12345")),
        Some("https://github.com/octo/repo/pull/42#discussion_r12345".to_string())
    );
    assert_eq!(
        github_pr_url(Some("octo/repo#42"), Some("github:ic:678")),
        Some("https://github.com/octo/repo/pull/42#issuecomment-678".to_string())
    );
    // An unrecognised event id — or none — leaves the reader at the PR.
    assert_eq!(
        github_pr_url(Some("octo/repo#42"), None),
        Some("https://github.com/octo/repo/pull/42".to_string())
    );
    assert_eq!(
        github_pr_url(Some("octo/repo#42"), Some("something-else")),
        Some("https://github.com/octo/repo/pull/42".to_string())
    );
}

/// A subject key that is not `owner/repo#N` yields no URL rather than a wrong
/// one.
#[test]
fn a_subject_key_that_is_not_owner_repo_number_yields_no_url() {
    assert_eq!(github_pr_url(Some("not-a-key"), None), None);
    assert_eq!(github_pr_url(Some("owner/repo#"), None), None);
    assert_eq!(github_pr_url(Some("owner/repo#abc"), None), None);
    assert_eq!(github_pr_url(Some("owner#42"), None), None);
    assert_eq!(github_pr_url(Some("a/b/c#42"), None), None);
    assert_eq!(github_pr_url(None, None), None);
}

/// The `message-sent` event carries the same origin as the snapshot bubble, so
/// a delta and a rebuild render one message the same way.
#[test]
fn the_message_sent_event_carries_the_same_origin_as_the_snapshot_bubble() {
    let mut msg = message(1, MessageType::User, "From Slack");
    msg.external_adapter = Some("slack".into());
    msg.external_source_name = Some("#eng".into());
    msg.binding_url = Some("https://acme.slack.com/archives/C1".into());
    msg.external_author = Some("ada".into());
    let bubble = message_bubble(
        &msg,
        &Value::Null,
        "2026-07-01T10:00:00.000Z",
        &OriginContext {
            frontend_url: "https://moatless.example",
            source_task_names: &HashMap::new(),
        },
    )
    .expect("a user bubble");
    assert!(bubble.get("origin").is_some());
    let payload = message_sent_payload("task-a", &bubble);
    assert_eq!(payload["origin"], bubble["origin"]);
}

/// A composer bubble has no origin, so neither does its event.
#[test]
fn a_plain_message_sent_event_carries_no_origin() {
    let bubble = message_bubble(
        &message(1, MessageType::User, "Hi"),
        &Value::Null,
        "2026-07-01T10:00:00.000Z",
        &OriginContext {
            frontend_url: "https://moatless.example",
            source_task_names: &HashMap::new(),
        },
    )
    .expect("a user bubble");
    assert!(bubble.get("origin").is_none());
    let payload = message_sent_payload("task-a", &bubble);
    assert!(payload.get("origin").is_none());
}

/// Only a person's bubble can carry an origin; the agent's own reply never
/// does, even on a row that happens to hold adapter columns.
#[test]
fn an_assistant_bubble_never_carries_an_origin() {
    let mut msg = message(1, MessageType::Assistant, "On it");
    msg.external_adapter = Some("slack".into());
    let bubble = message_bubble(
        &msg,
        &Value::Null,
        "2026-07-01T10:00:00.000Z",
        &OriginContext {
            frontend_url: "https://moatless.example",
            source_task_names: &HashMap::new(),
        },
    )
    .expect("an assistant bubble");
    assert!(bubble.get("origin").is_none());
}
