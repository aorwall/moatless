//! Cookie utilities for session management.

use axum::http::header;
use axum::http::HeaderMap;
use cookie::{Cookie, SameSite};
use crate::config::Config;

/// Extract a cookie value from the request headers.
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

/// Build a Set-Cookie header value for the session cookie.
pub fn build_session_cookie(config: &Config, session_id: &str, max_age_secs: u64) -> String {
    let mut builder = Cookie::build((config.session_cookie_name.clone(), session_id.to_string()))
        .http_only(true)
        .path("/")
        .same_site(SameSite::Lax)
        .max_age(cookie::time::Duration::seconds(max_age_secs as i64))
        .secure(config.is_production);

    if let Some(ref domain) = config.cookie_domain {
        builder = builder.domain(domain.clone());
    }

    builder.build().to_string()
}

/// Build a Set-Cookie header value that clears the session cookie.
pub fn build_clear_session_cookie(config: &Config) -> String {
    let mut builder = Cookie::build((config.session_cookie_name.clone(), ""))
        .http_only(true)
        .path("/")
        .same_site(SameSite::Lax)
        .max_age(cookie::time::Duration::ZERO)
        .secure(config.is_production);

    if let Some(ref domain) = config.cookie_domain {
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
        .secure(config.is_production)
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
        .secure(config.is_production)
        .build()
        .to_string()
}
