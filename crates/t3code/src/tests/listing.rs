//! Reading one listing row, and the reads that answer without touching the
//! database at all.

use super::fixtures::*;
use super::*;

/// An unfiltered browse is every Task the viewer may read, so the refusal has to
/// land before the query. `state()` sets no `expect_list_browsable`, and mockall
/// panics on a call it was never told to answer, so a refusal that stopped
/// short of the database is what a passing assertion means here.
#[tokio::test]
async fn a_browse_naming_neither_owner_nor_tag_never_reaches_the_database() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request("1", "threads.browse", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
    assert_eq!(
        output.frames[0]["exit"]["cause"][0]["defect"],
        "threads.browse requires an ownerUserId or a tag"
    );

    // A field holding only whitespace is the same request with a keystroke in
    // it, and `ConversationBrowse::new` trims so it lands on the same refusal.
    let output = connection
        .exchange(
            &request("2", "threads.browse", json!({ "tag": "   " })),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
}

/// The client asking is already showing the conversation. Answering `null`
/// leaves it showing what the thread subscription gave it; failing would
/// take a rendered transcript off the screen over a missing title.
#[tokio::test]
async fn a_listing_row_for_a_conversation_this_viewer_cannot_reach_is_null_not_an_error() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request("1", "threads.getShell", json!({ "threadId": "task-1" })),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(output.frames[0]["exit"]["value"]["thread"], Value::Null);
}

/// Every client encodes against the contract, so a request without one is
/// malformed rather than a conversation that is missing — and the two must
/// not answer alike, or a client bug reads as an empty deployment.
#[tokio::test]
async fn a_listing_row_asked_for_without_a_thread_id_is_a_failure() {
    let mut connection = connection();
    let output = connection
        .exchange(
            &request("1", "threads.getShell", json!({})),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Failure");
}

/// One character matches most of the corpus and says nothing, and it costs a
/// scan of every Message body to say so.
#[tokio::test]
async fn a_thread_search_too_short_to_mean_anything_never_reaches_the_database() {
    let mut connection = connection();
    // `state()` sets no `search_conversations` expectation, so the mock
    // panics if this reaches it — which is the assertion.
    let output = connection
        .exchange(
            &request("1", "orchestration.searchThreads", json!({ "query": "a" })),
            &state(),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert_eq!(output.frames[0]["exit"]["value"]["matches"], json!([]));
}
