//! `openidconnect`'s transport over a reqwest client bounded by the timeouts
//! `Config::http` holds.

use std::future::Future;
use std::pin::Pin;

use moatless_types::http::HttpTimeouts;
use openidconnect::{AsyncHttpClient, HttpClientError, HttpRequest, HttpResponse};

/// A reqwest client that follows no redirect: a discovery or token endpoint
/// that redirects is refused rather than followed.
pub(super) struct OidcHttp(reqwest::Client);

impl OidcHttp {
    #[expect(
        clippy::disallowed_methods,
        reason = "sets the timeouts `Config::http` holds"
    )]
    pub(super) fn new(timeouts: HttpTimeouts) -> Result<Self, reqwest::Error> {
        reqwest::Client::builder()
            .connect_timeout(timeouts.connect)
            .timeout(timeouts.total)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map(Self)
    }
}

impl<'c> AsyncHttpClient<'c> for OidcHttp {
    type Error = HttpClientError<reqwest::Error>;
    type Future = Pin<Box<dyn Future<Output = Result<HttpResponse, Self::Error>> + Send + 'c>>;

    fn call(&'c self, request: HttpRequest) -> Self::Future {
        Box::pin(async move {
            let response = self
                .0
                .execute(request.try_into().map_err(Box::new)?)
                .await
                .map_err(Box::new)?;
            let mut builder = openidconnect::http::Response::builder()
                .status(response.status())
                .version(response.version());
            for (name, value) in response.headers() {
                builder = builder.header(name, value);
            }
            let body = response.bytes().await.map_err(Box::new)?.to_vec();
            builder.body(body).map_err(HttpClientError::Http)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderValue, StatusCode, header};
    use axum::response::IntoResponse;

    #[tokio::test]
    async fn a_redirect_comes_back_as_the_response_with_its_headers() {
        let app = axum::Router::new().route(
            "/discovery",
            axum::routing::get(|| async {
                let mut response = (StatusCode::FOUND, "moved").into_response();
                response
                    .headers_mut()
                    .insert(header::LOCATION, HeaderValue::from_static("/elsewhere"));
                response
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stub");
        let addr = listener.local_addr().expect("stub addr");
        tokio::spawn(async move { axum::serve(listener, app).await.expect("serve stub") });

        let request = openidconnect::http::Request::get(format!("http://{addr}/discovery"))
            .body(Vec::new())
            .expect("request");
        let response =
            OidcHttp::new(moatless_core::config::HttpConfig::default().identity_provider)
                .expect("client")
                .call(request)
                .await
                .expect("response");

        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(response.headers()[header::LOCATION], "/elsewhere");
        assert_eq!(response.body(), b"moved");
    }
}
