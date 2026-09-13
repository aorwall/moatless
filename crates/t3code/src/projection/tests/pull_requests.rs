//! The pull requests a conversation row links, and the snapshot a refreshed
//! binding carries.

use super::fixtures::*;
use super::*;

/// A binding whose status blob has never been fetched, which is how one is
/// written. `status_read_at` still resolves, off `last_event_at`.
fn bound(repository: &str, number: i64) -> ConversationPullRequest {
    ConversationPullRequest {
        repository: repository.to_string(),
        number,
        url: format!("https://github.com/{repository}/pull/{number}"),
        title: None,
        base_ref: None,
        head_ref: None,
        state_kind: None,
        status_read_at: instant_at("2026-02-03T04:05:06Z"),
        linked_at: instant_at("2026-02-03T04:00:00Z"),
        origin: "connected_thread".to_string(),
        review_decision: None,
        mergeable_state: None,
    }
}

/// The same binding after a refresh, which fills every status field at once.
fn fetched(pr: ConversationPullRequest, state_kind: &str) -> ConversationPullRequest {
    ConversationPullRequest {
        title: Some(format!("Ship {}", pr.number)),
        base_ref: Some("main".to_string()),
        head_ref: Some(format!("feature/{}", pr.number)),
        state_kind: Some(state_kind.to_string()),
        ..pr
    }
}

/// The first link a row reports.
fn first_link(row: &Conversation) -> Value {
    one_thread(row)["pullRequests"][0].clone()
}

/// The link is the Task's binding, so it holds whatever branch the checkout is
/// on — including none at all, which is the ordinary case for a Task created
/// without picking one.
#[test]
fn a_thread_row_links_its_bound_pull_request_regardless_of_the_branch() {
    let mut row = conversation("task-a", "ws-a");
    row.branch = None;
    row.pull_requests = Some(Json(vec![bound("acme/web", 42)]));

    let link = first_link(&row);
    assert_eq!(link["host"], "github.com");
    assert_eq!(link["repository"], "acme/web");
    assert_eq!(link["number"], 42);
    assert_eq!(link["url"], "https://github.com/acme/web/pull/42");
    assert_eq!(link["linkedAt"], "2026-02-03T04:00:00.000Z");
    assert_eq!(link["stack"], Value::Null);
}

/// A client from before `threadPullRequests` reads the single reference, so the
/// row keeps answering it with the primary of the list.
#[test]
fn a_thread_row_also_links_the_primary_as_the_legacy_reference() {
    let mut row = conversation("task-a", "ws-a");
    row.pull_requests = Some(Json(vec![bound("acme/web", 42)]));

    let legacy = &one_thread(&row)["linkedPullRequest"];
    assert_eq!(legacy["projectId"], "ws-a");
    assert_eq!(legacy["repository"], "acme/web");
    assert_eq!(legacy["number"], 42);
    assert_eq!(legacy["url"], "https://github.com/acme/web/pull/42");
}

/// The query orders the list, so the row takes its first element as the primary
/// rather than choosing again on a shape that has lost the states the choice
/// was made from.
#[test]
fn a_thread_row_links_every_bound_pull_request_with_the_primary_first() {
    let mut row = conversation("task-a", "ws-a");
    row.pull_requests = Some(Json(vec![bound("acme/web", 7), bound("acme/web", 42)]));

    let thread = one_thread(&row);
    let links = thread["pullRequests"]
        .as_array()
        .expect("pullRequests is an array");
    assert_eq!(links.len(), 2);
    assert_eq!(links[0]["number"], 7);
    assert_eq!(links[1]["number"], 42);
    assert_eq!(thread["linkedPullRequest"]["number"], 7);
}

/// Moatless records the direction a Task and a subject met, so the CLI's own
/// binding is the only one T3 can call `created`.
#[test]
fn a_thread_row_reports_a_task_owned_binding_as_created_from_the_thread() {
    let mut row = conversation("task-a", "ws-a");
    row.pull_requests = Some(Json(vec![
        ConversationPullRequest {
            origin: "task_owned".to_string(),
            ..bound("acme/web", 42)
        },
        bound("acme/web", 7),
    ]));

    let thread = one_thread(&row);
    assert_eq!(thread["pullRequests"][0]["source"], "created");
    assert_eq!(thread["pullRequests"][1]["source"], "manual");
}

/// What the rows of the composer's pull-request menu and the sidebar's status
/// chip read, so neither has to ask `pullRequests.summary` for a binding this
/// deployment has already fetched.
#[test]
fn a_thread_row_carries_the_snapshot_a_refreshed_binding_recorded() {
    let mut row = conversation("task-a", "ws-a");
    row.pull_requests = Some(Json(vec![fetched(bound("acme/web", 42), "open")]));

    let snapshot = first_link(&row)["snapshot"].clone();
    assert_eq!(snapshot["title"], "Ship 42");
    assert_eq!(snapshot["state"], "open");
    assert_eq!(snapshot["isDraft"], false);
    assert_eq!(snapshot["baseBranch"], "main");
    assert_eq!(snapshot["headBranch"], "feature/42");
    assert_eq!(snapshot["syncedAt"], "2026-02-03T04:05:06.000Z");
}

/// `syncedAt` is when this deployment read GitHub. The blob records no instant
/// at which GitHub itself changed the pull request, so `updatedAt` stays null
/// rather than repeating the read.
#[test]
fn a_snapshot_reports_no_update_instant_of_its_own() {
    let mut row = conversation("task-a", "ws-a");
    row.pull_requests = Some(Json(vec![fetched(bound("acme/web", 42), "open")]));

    assert_eq!(first_link(&row)["snapshot"]["updatedAt"], Value::Null);
}

/// GitHub's spellings reach the wire as T3's, and a state neither side names is
/// reported as `unknown` rather than dropped.
#[test]
fn a_snapshot_translates_the_review_and_merge_verdicts() {
    let mut row = conversation("task-a", "ws-a");
    row.pull_requests = Some(Json(vec![ConversationPullRequest {
        review_decision: Some("changes_requested".to_string()),
        mergeable_state: Some("dirty".to_string()),
        ..fetched(bound("acme/web", 42), "open")
    }]));

    let snapshot = first_link(&row)["snapshot"].clone();
    assert_eq!(snapshot["reviewDecision"], "changes-requested");
    assert_eq!(snapshot["mergeability"], "conflicting");
}

/// A draft reports the same `state` as an open pull request, because the
/// contract has no fourth value, so `isDraft` is what colours it grey.
#[test]
fn a_thread_row_reports_a_draft_binding_as_an_open_draft() {
    let mut row = conversation("task-a", "ws-a");
    row.pull_requests = Some(Json(vec![fetched(bound("acme/web", 42), "draft")]));

    let snapshot = first_link(&row)["snapshot"].clone();
    assert_eq!(snapshot["state"], "open");
    assert_eq!(snapshot["isDraft"], true);
}

/// A blob holding the branches but neither a title nor a state is one
/// `persisted_status_is_complete` treats as fetched, so no refresh ever repairs
/// it.
///
/// The row carries these two stand-ins instead of buying them with a round
/// trip.
#[test]
fn a_thread_row_stands_in_for_a_title_and_a_state_the_blob_never_recorded() {
    let mut row = conversation("task-a", "ws-a");
    row.pull_requests = Some(Json(vec![ConversationPullRequest {
        title: None,
        state_kind: None,
        ..fetched(bound("acme/web", 42), "open")
    }]));

    let snapshot = first_link(&row)["snapshot"].clone();
    assert_eq!(snapshot["title"], "acme/web#42");
    assert_eq!(snapshot["state"], "open");
    assert_eq!(snapshot["isDraft"], false);
}

/// A binding is written with its blob NULL, filled in only by a refresh no
/// listing can trigger, so the link travels without a snapshot rather than a
/// state nobody read off GitHub.
///
/// `pullRequests.summary` is what the client falls back to, and what refreshes
/// it.
#[test]
fn a_thread_row_carries_no_snapshot_for_a_binding_never_fetched() {
    let mut row = conversation("task-a", "ws-a");
    row.pull_requests = Some(Json(vec![bound("acme/web", 42)]));

    let link = first_link(&row);
    assert_eq!(link["number"], 42);
    assert_eq!(link["snapshot"], Value::Null);
}

#[test]
fn a_thread_row_with_no_binding_links_no_pull_request() {
    let thread = one_thread(&conversation("task-a", "ws-a"));
    assert_eq!(thread["linkedPullRequest"], Value::Null);
    assert_eq!(thread["pullRequests"], json!([]));
}

/// A reference is written whole, so a half-filled one is a bug upstream of here
/// rather than something to put on the wire.
#[test]
fn a_thread_row_drops_a_pull_request_it_cannot_name() {
    let mut row = conversation("task-a", "ws-a");
    row.pull_requests = Some(Json(vec![
        ConversationPullRequest {
            repository: String::new(),
            ..bound("acme/web", 42)
        },
        bound("acme/web", 7),
    ]));

    let thread = one_thread(&row);
    assert_eq!(thread["pullRequests"].as_array().map(Vec::len), Some(1));
    assert_eq!(thread["linkedPullRequest"]["number"], 7);
}
