//! Cookie utilities for session management.

use axum::http::HeaderMap;
use axum::http::header;
use cookie::{Cookie, SameSite};
use moatless_core::config::Config;

pub const CSRF_COOKIE_NAME: &str = "moatless_csrf";

/// Extract a cookie value from the request headers (returns the first match).
pub fn get_cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie_header = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in cookie_header.split(';') {
        if let Ok(c) = Cookie::parse(pair.trim()) {
            if c.name() == name {
                return Some(c.value().to_string());
            }
        }
    }
    None
}

/// Extract all cookie values matching `name` from the request headers.
///
/// Browsers may send duplicate cookies when the domain or path changed between
/// deployments. This returns every matching value so callers can try each one.
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

/// Build a Set-Cookie header value for the session cookie.
pub fn build_session_cookie(config: &Config, token: &str, max_age_secs: u64) -> String {
    let mut builder = Cookie::build((config.auth.session_cookie_name.clone(), token.to_string()))
        .http_only(true)
        .path("/")
        .same_site(SameSite::Lax)
        .max_age(cookie::time::Duration::seconds(max_age_secs as i64))
        .secure(config.auth.is_production);

    if let Some(ref domain) = config.auth.cookie_domain {
        builder = builder.domain(domain.clone());
    }

    builder.build().to_string()
}

/// Build a Set-Cookie header value that clears the session cookie.
pub fn build_clear_session_cookie(config: &Config) -> String {
    let mut builder = Cookie::build((config.auth.session_cookie_name.clone(), ""))
        .http_only(true)
        .path("/")
        .same_site(SameSite::Lax)
        .max_age(cookie::time::Duration::ZERO)
        .secure(config.auth.is_production);

    if let Some(ref domain) = config.auth.cookie_domain {
        builder = builder.domain(domain.clone());
    }

    builder.build().to_string()
}

/// Build the readable half of the double-submit CSRF token. Its digest is
/// bound to the authoritative auth-session row.
pub fn build_csrf_cookie(config: &Config, token: &str, max_age_secs: u64) -> String {
    let mut builder = Cookie::build((CSRF_COOKIE_NAME, token.to_string()))
        .http_only(false)
        .path("/")
        .same_site(SameSite::Lax)
        .max_age(cookie::time::Duration::seconds(max_age_secs as i64))
        .secure(config.auth.is_production);
    if let Some(ref domain) = config.auth.cookie_domain {
        builder = builder.domain(domain.clone());
    }
    builder.build().to_string()
}

pub fn build_clear_csrf_cookie(config: &Config) -> String {
    let mut builder = Cookie::build((CSRF_COOKIE_NAME, ""))
        .http_only(false)
        .path("/")
        .same_site(SameSite::Lax)
        .max_age(cookie::time::Duration::ZERO)
        .secure(config.auth.is_production);
    if let Some(ref domain) = config.auth.cookie_domain {
        builder = builder.domain(domain.clone());
    }
    builder.build().to_string()
}

/// Build a Set-Cookie header value for the OAuth state cookie.
pub fn build_oauth_state_cookie(config: &Config, state: &str) -> String {
    Cookie::build(("oauth_state", state.to_string()))
        .http_only(true)
        .path("/")
        .same_site(SameSite::Lax)
        .max_age(cookie::time::Duration::seconds(600)) // 10 minutes
        .secure(config.auth.is_production)
        .build()
        .to_string()
}

/// Build a Set-Cookie header value that clears the OAuth state cookie.
pub fn build_clear_oauth_state_cookie(config: &Config) -> String {
    Cookie::build(("oauth_state", ""))
        .http_only(true)
        .path("/")
        .same_site(SameSite::Lax)
        .max_age(cookie::time::Duration::ZERO)
        .secure(config.auth.is_production)
        .build()
        .to_string()
}
