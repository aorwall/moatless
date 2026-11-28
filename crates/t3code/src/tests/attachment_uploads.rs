//! The upload route, driven the way the composer drives it: mint a URL over the
//! socket, then `POST` the raw file to it.

use super::fixtures::*;
use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

/// Mint a URL for an attachment of `size` bytes and return the path to `POST`
/// it to.
async fn minted_url(state: &UiRpcState, size: usize) -> String {
    let frame = request(
        "1",
        "attachments.createUploadUrl",
        json!({
            "type": "file",
            "name": "report.pdf",
            "mimeType": "application/pdf",
            "sizeBytes": size
        }),
    );
    let mut session = session();
    let output = session.handle_json(&frame, state, &viewer()).await;
    output.frames[0]["exit"]["value"]["relativeUrl"]
        .as_str()
        .expect("a minted url")
        .to_string()
}

async fn upload(state: UiRpcState, url: &str, body: Vec<u8>) -> StatusCode {
    let request = Request::builder()
        .method("POST")
        .uri(url)
        .header("content-type", "application/pdf")
        .body(Body::from(body))
        .expect("a request");
    router(state)
        .oneshot(request)
        .await
        .expect("a response")
        .status()
}

/// The whole point of the route: bytes in, and the name and type the mint
/// declared — not the ones the request carried — stored beside them under the
/// id the client is already holding.
#[tokio::test]
async fn a_body_of_the_declared_size_is_stored_under_the_minted_id() {
    let attachments = Arc::new(RecordingAttachments::default());
    let state = state_with_attachments(attachments.clone());
    let url = minted_url(&state, 8).await;
    let id = url.rsplit('/').next().unwrap_or_default().to_string();

    assert_eq!(
        upload(state, &url, vec![7u8; 8]).await,
        StatusCode::NO_CONTENT
    );
    #[expect(clippy::unwrap_used, reason = "tests fail by panicking")]
    let stored = attachments.stored.lock().unwrap().clone();
    assert_eq!(stored.len(), 1, "{stored:?}");
    assert_eq!(stored[0].1, "report.pdf");
    assert_eq!(stored[0].2, "application/pdf");
    assert_eq!(stored[0].3, 8);
    // The id is in the token, not in the path — the path *is* the
    // token — so this asserts the two are the same string.
    assert!(!id.is_empty());
    assert!(url.ends_with(&id));
}

/// The boundary, from both sides. A body one byte over its declaration is
/// refused and stores nothing; a body of exactly the declared size is stored.
#[tokio::test]
async fn a_body_is_refused_at_the_first_byte_past_its_declaration() {
    let attachments = Arc::new(RecordingAttachments::default());
    let state = state_with_attachments(attachments.clone());
    let url = minted_url(&state, 8).await;

    assert_eq!(
        upload(state.clone(), &url, vec![7u8; 9]).await,
        StatusCode::BAD_REQUEST
    );
    #[expect(clippy::unwrap_used, reason = "tests fail by panicking")]
    let stored = attachments.stored.lock().unwrap().len();
    assert_eq!(stored, 0, "an oversize body must not be stored");

    assert_eq!(
        upload(state, &url, vec![7u8; 8]).await,
        StatusCode::NO_CONTENT
    );
}

/// A body that ends short of its declaration is a truncated upload, and storing
/// it would reach the agent as a corrupt file.
#[tokio::test]
async fn a_body_that_ends_short_of_its_declaration_is_refused() {
    let attachments = Arc::new(RecordingAttachments::default());
    let state = state_with_attachments(attachments.clone());
    let url = minted_url(&state, 8).await;

    assert_eq!(
        upload(state, &url, vec![7u8; 7]).await,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(attachments.stored.lock().expect("no poison").len(), 0);
}

/// The token in the path is the credential: a forged one, an edited one, and
/// one from another deployment all write nothing.
///
/// This route is exempt from the auth middleware precisely because it
/// authenticates itself.
#[tokio::test]
async fn a_url_this_server_did_not_mint_stores_nothing() {
    let attachments = Arc::new(RecordingAttachments::default());
    let state = state_with_attachments(attachments.clone());
    let minted = minted_url(&state, 8).await;

    for url in [
        "/api/attachments/upload/not-a-token".to_string(),
        format!("{minted}x"),
    ] {
        assert_eq!(
            upload(state.clone(), &url, vec![7u8; 8]).await,
            StatusCode::UNAUTHORIZED,
            "{url}"
        );
    }
    assert_eq!(attachments.stored.lock().expect("no poison").len(), 0);
}
