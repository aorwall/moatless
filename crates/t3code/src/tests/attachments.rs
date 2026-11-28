//! Minting an upload URL and deleting an attachment, against a recording store.

use super::fixtures::*;
use super::*;

/// What the composer gets back before it moves any bytes: the id it will name
/// on the turn, and the URL it will `POST` to.
#[tokio::test]
async fn create_upload_url_answers_an_id_and_a_url_under_the_upload_prefix() {
    let frame = request(
        "1",
        "attachments.createUploadUrl",
        json!({ "type": "file", "name": "report.pdf", "mimeType": "application/pdf", "sizeBytes": 2048 }),
    );
    let mut session = session();
    let output = session.handle_json(&frame, &state(), &viewer()).await;
    let value = &output.frames[0]["exit"]["value"];

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    assert!(
        value["attachmentId"]
            .as_str()
            .is_some_and(|id| !id.is_empty())
    );
    // `AttachmentCreateUploadUrlResult.relativeUrl`'s pattern; the client
    // drops a mint that fails it, and the upload never starts.
    let token = value["relativeUrl"]
        .as_str()
        .and_then(|url| url.strip_prefix("/api/attachments/upload/"))
        .unwrap_or_default();
    assert!(
        !token.is_empty()
            && token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')),
        "{value}"
    );
    assert!(value["expiresAt"].as_i64().is_some_and(|at| at > 0));
}

/// A payload outside the union never becomes an upload URL. The client's own
/// schema refuses these before they are sent, so this is a malformed request
/// rather than one of the method's declared outcomes.
#[tokio::test]
async fn create_upload_url_refuses_a_payload_outside_the_union() {
    let frame = request(
        "1",
        "attachments.createUploadUrl",
        json!({ "type": "image", "name": "a.tiff", "mimeType": "image/tiff", "sizeBytes": 10 }),
    );
    let mut session = session();
    let output = session.handle_json(&frame, &state(), &viewer()).await;

    assert_eq!(output.frames[0]["exit"]["cause"][0]["_tag"], "Die");
}

/// The composer deletes an attachment it abandoned — a cancel, a retry, a
/// discarded draft — without knowing whether the upload ever landed, so this
/// succeeds either way and the id is what has to reach the store.
#[tokio::test]
async fn delete_reaches_the_store_with_the_id_it_was_given() {
    let attachments = Arc::new(RecordingAttachments::default());
    let frame = request(
        "1",
        "attachments.delete",
        json!({ "attachmentId": "6b1f7b2e-0000-4000-8000-000000000001" }),
    );
    let mut session = session();
    let output = session
        .handle_json(
            &frame,
            &state_with_attachments(attachments.clone()),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
    #[expect(clippy::unwrap_used, reason = "tests fail by panicking")]
    let deleted = attachments.deleted.lock().unwrap().clone();
    assert_eq!(deleted, vec!["6b1f7b2e-0000-4000-8000-000000000001"]);
}

/// Somebody else's attachment is, from here, one that is not there: answering
/// differently would make this a way to ask whether an id exists.
#[tokio::test]
async fn delete_of_an_attachment_this_viewer_may_not_touch_still_succeeds() {
    let attachments = Arc::new(RecordingAttachments {
        refuse: Some(ports::AttachmentStoreError::Forbidden),
        ..RecordingAttachments::default()
    });
    let frame = request("1", "attachments.delete", json!({ "attachmentId": "abc" }));
    let mut session = session();
    let output = session
        .handle_json(&frame, &state_with_attachments(attachments), &viewer())
        .await;

    assert_eq!(output.frames[0]["exit"]["_tag"], "Success");
}

/// An id that could leave the route it becomes a segment of never reaches the
/// store.
#[tokio::test]
async fn delete_of_something_that_is_not_an_id_never_reaches_the_store() {
    let attachments = Arc::new(RecordingAttachments::default());
    let frame = request(
        "1",
        "attachments.delete",
        json!({ "attachmentId": "../secrets" }),
    );
    let mut session = session();
    let output = session
        .handle_json(
            &frame,
            &state_with_attachments(attachments.clone()),
            &viewer(),
        )
        .await;

    assert_eq!(output.frames[0]["exit"]["cause"][0]["_tag"], "Die");
    #[expect(clippy::unwrap_used, reason = "tests fail by panicking")]
    let deleted = attachments.deleted.lock().unwrap().len();
    assert_eq!(deleted, 0);
}
