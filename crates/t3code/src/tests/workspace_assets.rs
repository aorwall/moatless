//! The route a browser frame fetches a workspace file from.

use super::fixtures::*;
use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

async fn text(response: Response) -> String {
    let body = axum::body::to_bytes(response.into_body(), MAX_FRAME_BYTES)
        .await
        .unwrap();
    String::from_utf8(body.to_vec()).unwrap()
}

/// A minted URL is an address, not a capability: the Task behind it is re-read
/// through the follow-and-access filter each fetch, so an unfollowed viewer is
/// told it does not exist.
#[tokio::test]
async fn a_conversation_this_viewer_does_not_follow_serves_no_file() {
    let mut request = Request::builder()
        .uri("/api/assets/t-1/docs/report.html")
        .body(Body::empty())
        .unwrap();
    request.extensions_mut().insert(viewer());

    let response = router(state()).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    // The route is mounted and refused it, rather than never matching.
    assert_eq!(
        text(response).await,
        crate::workspace::WorkspaceAccessError::Unaddressable.message()
    );
}
