//! Where a finished connect flow sends the browser, and what it tells the page
//! it lands on.
//!
//! Shared by [`super::github_connect`] and [`super::forgejo_connect`]: both run
//! an authorization flow against a session that already exists, and both come
//! back to a page the person opened, so neither may answer with a JSON error
//! body and both vet the URL they were handed.

use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use moatless_types::auth::AllowedOrigin;
use tracing::warn;

/// The origin part of an absolute `http(s)` URL — scheme, host and port.
fn origin_of(url: &str) -> Option<&str> {
    let scheme_end = url.find("://")? + 3;
    let rest = url.get(scheme_end..)?;
    let host_len = rest.find('/').unwrap_or(rest.len());
    // A zero-length host would make every origin comparison pass against
    // `https://`, so an authority is required rather than merely permitted.
    (host_len > 0).then(|| &url[..scheme_end + host_len])
}

/// Where a finished connect may send the browser.
///
/// A request names its own return URL, so it is checked against the same
/// origins the CSRF middleware trusts for cookie-authenticated writes; anything
/// else is an open redirect and falls back to the configured frontend. One list
/// covers both because a client that may drive writes with this session is
/// already a client this session's credential work belongs to.
pub(super) fn approved_return_url(
    frontend_url: &str,
    trusted_origins: &[AllowedOrigin],
    requested: Option<&str>,
) -> String {
    let Some(requested) = requested.filter(|url| !url.is_empty()) else {
        return frontend_url.to_string();
    };
    let Some(origin) = origin_of(requested) else {
        return frontend_url.to_string();
    };
    let trusted = origin_of(frontend_url) == Some(origin)
        || trusted_origins
            .iter()
            .any(|allowed| allowed.matches(origin));
    if trusted {
        requested.to_string()
    } else {
        warn!(%origin, "Rejected a connect return URL from an untrusted origin");
        frontend_url.to_string()
    }
}

/// Load `return_url` with `param={outcome}`, ahead of any fragment.
///
/// A fragment is not part of the query, so appending after one puts the
/// parameter where `location.search` cannot see it — and reporting the outcome
/// is the only thing it is for.
pub(super) fn with_outcome(return_url: &str, param: &str, outcome: &str) -> String {
    let (base, fragment) = match return_url.split_once('#') {
        Some((base, fragment)) => (base, Some(fragment)),
        None => (return_url, None),
    };
    let separator = if base.contains('?') { '&' } else { '?' };
    match fragment {
        Some(fragment) => format!("{base}{separator}{param}={outcome}#{fragment}"),
        None => format!("{base}{separator}{param}={outcome}"),
    }
}

pub(super) fn redirect(location: &str, extra_cookie: Option<String>) -> Response {
    let mut headers = HeaderMap::new();
    if let Some(value) = extra_cookie.and_then(|cookie| cookie.parse().ok()) {
        headers.append(header::SET_COOKIE, value);
    }
    (StatusCode::FOUND, headers, [(header::LOCATION, location)]).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRONTEND: &str = "https://app.example";

    fn return_url(trusted: &[&str], requested: &str) -> String {
        let trusted: Vec<AllowedOrigin> = trusted.iter().map(|o| AllowedOrigin::parse(o)).collect();
        approved_return_url(FRONTEND, &trusted, Some(requested))
    }

    #[test]
    fn the_frontend_is_allowed_without_being_listed_as_trusted() {
        assert_eq!(
            return_url(&[], "https://app.example/settings/account"),
            "https://app.example/settings/account"
        );
    }

    #[test]
    fn a_second_client_is_allowed_once_its_origin_is_trusted() {
        assert_eq!(
            return_url(
                &["https://t3.example"],
                "https://t3.example/settings/account"
            ),
            "https://t3.example/settings/account"
        );
    }

    #[test]
    fn an_untrusted_origin_returns_to_the_frontend_instead() {
        // Suffix-matching a trusted host is the shape that turns a return URL
        // into an open redirect, so it is the one worth pinning.
        for hostile in [
            "https://evil.example/steal",
            "https://t3.example.evil.example/steal",
            "https://app.example.evil.example/steal",
            "//evil.example/steal",
        ] {
            assert_eq!(
                return_url(&["https://t3.example"], hostile),
                FRONTEND,
                "{hostile} must not be a return target"
            );
        }
    }

    #[test]
    fn a_wildcard_trusted_origin_covers_its_subdomains_only() {
        assert_eq!(
            return_url(&["https://*.example.com"], "https://t3.example.com/x"),
            "https://t3.example.com/x"
        );
        assert_eq!(
            return_url(&["https://*.example.com"], "https://example.com/x"),
            FRONTEND
        );
    }

    #[test]
    fn no_requested_url_returns_to_the_frontend() {
        assert_eq!(approved_return_url(FRONTEND, &[], None), FRONTEND);
        assert_eq!(approved_return_url(FRONTEND, &[], Some("")), FRONTEND);
    }

    #[test]
    fn the_outcome_is_appended_to_a_url_that_already_has_a_query() {
        assert_eq!(
            with_outcome("https://t3.example/s", "githubConnect", "connected"),
            "https://t3.example/s?githubConnect=connected"
        );
        assert_eq!(
            with_outcome("https://t3.example/s?tab=git", "githubConnect", "connected"),
            "https://t3.example/s?tab=git&githubConnect=connected"
        );
    }

    #[test]
    fn the_outcome_lands_in_the_query_of_a_url_with_a_fragment() {
        // Appending after the `#` puts it where `location.search` cannot see
        // it, and being read is the whole point of the parameter.
        assert_eq!(
            with_outcome("https://t3.example/s#git", "githubConnect", "connected"),
            "https://t3.example/s?githubConnect=connected#git"
        );
        assert_eq!(
            with_outcome("https://t3.example/s?tab=git#top", "forgejoConnect", "denied"),
            "https://t3.example/s?tab=git&forgejoConnect=denied#top"
        );
    }
}
