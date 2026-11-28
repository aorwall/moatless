//! Cookie utilities for session management.

use axum::http::HeaderMap;
use axum::http::header;
use cookie::time::Duration;
use cookie::{Cookie, CookieBuilder, SameSite};
use moatless_core::config::Config;

pub const CSRF_COOKIE_NAME: &str = "moatless_csrf";

/// Extract a cookie value from the request headers (returns the first match).
pub fn get_cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in cookie_header.split(';') {
        if let Ok(c) = Cookie::parse(pair.trim())
            && c.name() == name
        {
            return Some(c.value().to_string());
        }
    }
    None
}

/// Extracts all cookie values matching `name` from the request headers.
///
/// Browsers may send duplicates when the domain or path changed between
/// deployments; this returns every match so callers can try each.
pub fn get_all_cookie_values(headers: &HeaderMap, name: &str) -> Vec<String> {
    let Some(cookie_header) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) else {
        return Vec::new();
    };
    cookie_header
        .split(';')
        .filter_map(|pair| {
            let c = Cookie::parse(pair.trim()).ok()?;
            if c.name() == name {
                Some(c.value().to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Cookie shape here: root path, `Lax`, `Secure` in prod, `cookie_domain`.
/// A clear must match its set's domain and path, or it leaves a second
/// cookie instead of removing the live one.
///
/// A GitHub App's callback returns on the frontend's host; a host-only
/// `cookie_domain` fails CSRF there in a two-host deployment.
fn build_cookie(
    config: &Config,
    name: &str,
    value: &str,
    max_age: Duration,
) -> CookieBuilder<'static> {
    let mut builder = Cookie::build((name.to_string(), value.to_string()))
        .path("/")
        .same_site(SameSite::Lax)
        .max_age(max_age)
        .secure(config.auth.is_production);

    if let Some(ref domain) = config.auth.cookie_domain {
        builder = builder.domain(domain.clone());
    }

    builder
}

/// Build a Set-Cookie header value for the session cookie.
pub fn build_session_cookie(config: &Config, token: &str, max_age_secs: u64) -> String {
    build_cookie(
        config,
        &config.auth.session_cookie_name,
        token,
        Duration::seconds(max_age_secs as i64),
    )
    .http_only(true)
    .build()
    .to_string()
}

/// Build a Set-Cookie header value that clears the session cookie.
pub fn build_clear_session_cookie(config: &Config) -> String {
    build_cookie(config, &config.auth.session_cookie_name, "", Duration::ZERO)
        .http_only(true)
        .build()
        .to_string()
}

/// Build the readable half of the double-submit CSRF token. Its digest is bound
/// to the authoritative auth-session row.
pub fn build_csrf_cookie(config: &Config, token: &str, max_age_secs: u64) -> String {
    build_cookie(
        config,
        CSRF_COOKIE_NAME,
        token,
        Duration::seconds(max_age_secs as i64),
    )
    // The client half of the double submit: the browser has to read it back.
    .http_only(false)
    .build()
    .to_string()
}

pub fn build_clear_csrf_cookie(config: &Config) -> String {
    build_cookie(config, CSRF_COOKIE_NAME, "", Duration::ZERO)
        .http_only(false)
        .build()
        .to_string()
}

/// Build a Set-Cookie header value for the OAuth state cookie. Lives 10
/// minutes.
pub fn build_oauth_state_cookie(config: &Config, state: &str) -> String {
    build_cookie(config, "oauth_state", state, Duration::seconds(600))
        .http_only(true)
        .build()
        .to_string()
}

/// Build a Set-Cookie header value that clears the OAuth state cookie.
pub fn build_clear_oauth_state_cookie(config: &Config) -> String {
    build_cookie(config, "oauth_state", "", Duration::ZERO)
        .http_only(true)
        .build()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config_with_domain(domain: Option<&str>) -> Config {
        let mut config = Config::test_minimal();
        config.auth.cookie_domain = domain.map(str::to_string);
        config
    }

    #[test]
    fn the_oauth_state_cookie_spans_the_configured_domain() {
        let config = config_with_domain(Some(".example.com"));
        assert!(
            build_oauth_state_cookie(&config, "nonce").contains("Domain=example.com"),
            "the state cookie must reach the callback host"
        );
    }

    #[test]
    fn clearing_the_oauth_state_cookie_names_the_same_domain() {
        let config = config_with_domain(Some(".example.com"));
        assert!(build_clear_oauth_state_cookie(&config).contains("Domain=example.com"));
    }

    #[test]
    fn only_the_readable_half_of_the_double_submit_is_reachable_from_js() {
        // `build_cookie` leaves `http_only` to each caller, so the default it
        // hands a new builder is "reachable from JS".
        let config = config_with_domain(None);
        assert!(build_session_cookie(&config, "token", 60).contains("HttpOnly"));
        assert!(!build_csrf_cookie(&config, "token", 60).contains("HttpOnly"));
    }

    #[test]
    fn a_deployment_with_one_host_sets_no_domain() {
        let config = config_with_domain(None);
        assert!(!build_oauth_state_cookie(&config, "nonce").contains("Domain="));
        assert!(!build_clear_oauth_state_cookie(&config).contains("Domain="));
    }
}
