//! Classifying a GitHub REST or GraphQL rejection, so a quota refusal carries
//! its reset time and a permission refusal is not retried as transient.

use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use reqwest::header::{HeaderMap, RETRY_AFTER};

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};

const RATE_LIMIT_REMAINING: &str = "x-ratelimit-remaining";
const RATE_LIMIT_RESET: &str = "x-ratelimit-reset";

/// The error for a non-success answer to `operation`.
///
/// A 429, or a 403 that GitHub marks as a primary or secondary rate limit, is
/// `RateLimited`; any other 403 is `AccessDenied`; the rest stay `AdapterApi`.
pub(crate) fn rejection(
    operation: &str,
    status: StatusCode,
    headers: &HeaderMap,
    body: &str,
) -> AdapterError {
    let detail = format!("{operation} returned {status}: {body}");
    if is_throttle(status, headers, body) {
        return SharedAdapterError::RateLimited {
            detail,
            retry_at: retry_at(headers, Utc::now()),
        }
        .into();
    }
    if status == StatusCode::FORBIDDEN {
        return SharedAdapterError::AccessDenied(detail).into();
    }
    SharedAdapterError::AdapterApi(detail).into()
}

/// GitHub's GraphQL answers a spent quota with 200 and an error of type
/// `RATE_LIMITED`, so the status alone cannot classify it.
pub(crate) fn graphql_rate_limited(errors: &serde_json::Value) -> bool {
    errors.as_array().is_some_and(|errors| {
        errors
            .iter()
            .any(|error| error["type"].as_str() == Some("RATE_LIMITED"))
    })
}

/// When the quota returns, read from the headers GitHub documents: a
/// `retry-after` in seconds, else `x-ratelimit-reset` in epoch seconds once
/// `x-ratelimit-remaining` is zero.
pub(crate) fn retry_at(headers: &HeaderMap, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if let Some(seconds) = header(headers, RETRY_AFTER.as_str()).and_then(|v| v.parse::<i64>().ok())
    {
        return Some(now + chrono::Duration::seconds(seconds.max(0)));
    }
    if header(headers, RATE_LIMIT_REMAINING) != Some("0") {
        return None;
    }
    header(headers, RATE_LIMIT_RESET)
        .and_then(|v| v.parse::<i64>().ok())
        .and_then(|epoch| DateTime::from_timestamp(epoch, 0))
}

fn is_throttle(status: StatusCode, headers: &HeaderMap, body: &str) -> bool {
    if status == StatusCode::TOO_MANY_REQUESTS {
        return true;
    }
    status == StatusCode::FORBIDDEN
        && (headers.contains_key(RETRY_AFTER)
            || header(headers, RATE_LIMIT_REMAINING) == Some("0")
            || body.to_ascii_lowercase().contains("rate limit"))
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, value.parse().unwrap());
        }
        map
    }

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_793_000_000, 0).unwrap()
    }

    /// The 2026-11-02 answer: a primary limit, with its reset in the headers.
    #[test]
    fn a_primary_limit_is_rate_limited_until_its_reset() {
        let reset = now() + chrono::Duration::minutes(37);
        let reset_epoch = reset.timestamp().to_string();
        let map = headers(&[
            (RATE_LIMIT_REMAINING, "0"),
            (RATE_LIMIT_RESET, reset_epoch.as_str()),
        ]);
        let error = rejection(
            "GitHub get_pr",
            StatusCode::FORBIDDEN,
            &map,
            r#"{"message":"API rate limit exceeded for user ID 609489."}"#,
        );
        let AdapterError::Shared(SharedAdapterError::RateLimited { detail, retry_at }) = error
        else {
            panic!("a spent quota is rate limited: {error:?}");
        };
        assert!(detail.contains("API rate limit exceeded"));
        assert_eq!(retry_at, Some(reset));
    }

    #[test]
    fn a_secondary_limit_waits_its_retry_after() {
        let map = headers(&[("retry-after", "90")]);
        assert_eq!(
            retry_at(&map, now()),
            Some(now() + chrono::Duration::seconds(90))
        );
        let error = rejection(
            "GitHub post_comment",
            StatusCode::FORBIDDEN,
            &map,
            "You have exceeded a secondary rate limit",
        );
        assert!(matches!(
            error,
            AdapterError::Shared(SharedAdapterError::RateLimited { .. })
        ));
    }

    /// A secondary limit may name no time at all; the delivery policy then
    /// supplies the delay.
    #[test]
    fn a_limit_without_timing_headers_names_no_reset() {
        let error = rejection(
            "GitHub get_pr",
            StatusCode::TOO_MANY_REQUESTS,
            &HeaderMap::new(),
            "",
        );
        assert!(matches!(
            error,
            AdapterError::Shared(SharedAdapterError::RateLimited { retry_at: None, .. })
        ));
        // A reset with quota left is the ordinary header on every answer, not
        // a refusal's.
        let map = headers(&[
            (RATE_LIMIT_REMAINING, "12"),
            (RATE_LIMIT_RESET, "1793000600"),
        ]);
        assert_eq!(retry_at(&map, now()), None);
    }

    #[test]
    fn a_permission_refusal_is_access_denied() {
        let map = headers(&[(RATE_LIMIT_REMAINING, "4100")]);
        let error = rejection(
            "GitHub get_pr",
            StatusCode::FORBIDDEN,
            &map,
            r#"{"message":"Resource not accessible by integration"}"#,
        );
        assert!(matches!(
            error,
            AdapterError::Shared(SharedAdapterError::AccessDenied(_))
        ));
    }

    #[test]
    fn other_rejections_stay_provider_errors() {
        let error = rejection(
            "GitHub get_pr",
            StatusCode::BAD_GATEWAY,
            &HeaderMap::new(),
            "",
        );
        assert!(matches!(
            error,
            AdapterError::Shared(SharedAdapterError::AdapterApi(_))
        ));
    }

    #[test]
    fn graphql_reports_its_limit_in_the_errors_array() {
        let limited =
            serde_json::json!([{ "type": "RATE_LIMITED", "message": "API rate limit exceeded" }]);
        assert!(graphql_rate_limited(&limited));
        let forbidden = serde_json::json!([{ "type": "FORBIDDEN" }]);
        assert!(!graphql_rate_limited(&forbidden));
    }
}
