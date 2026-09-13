//! The JSON read routes negotiate compression; the socket does not.

use super::fixtures::*;
use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

async fn shell(accept_encoding: Option<&str>) -> Response {
    let mut builder = Request::builder().uri("/api/orchestration/shell");
    if let Some(encoding) = accept_encoding {
        builder = builder.header("accept-encoding", encoding);
    }
    let mut request = builder.body(Body::empty()).unwrap();
    request.extensions_mut().insert(viewer());
    router(state()).oneshot(request).await.unwrap()
}

/// A client that offers gzip gets it: the layer is wired to this route, which
/// is the ~5x saving on the thread bootstrap the client pulls over HTTP for
/// exactly this reason.
#[tokio::test]
async fn a_client_that_offers_gzip_gets_a_gzipped_body() {
    let response = shell(Some("gzip")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_ENCODING)
            .and_then(|value| value.to_str().ok()),
        Some("gzip")
    );
}

/// A client that offers nothing still gets a readable body: compression is
/// negotiated, never forced on a client that cannot decode it.
#[tokio::test]
async fn a_client_that_offers_no_encoding_gets_an_uncompressed_body() {
    let response = shell(None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(header::CONTENT_ENCODING).is_none());
}
