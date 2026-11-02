//! Classifying a GitHub REST or GraphQL rejection, so a quota refusal carries
//! its reset time and a permission refusal is not retried as transient.

use chrono::Utc;
use moatless_core::github_client::github_throttle;
use reqwest::StatusCode;
use reqwest::header::HeaderMap;

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};

/// The error for a non-success answer to `operation`.
///
/// A rate limit by [`github_throttle`] is `RateLimited`; any other 403 is
/// `AccessDenied`; the rest stay `AdapterApi`.
pub(crate) fn rejection(
    operation: &str,
    status: StatusCode,
    headers: &HeaderMap,
    body: &str,
) -> AdapterError {
    let detail = format!("{operation} returned {status}: {body}");
    if let Some(throttle) = github_throttle(status, headers, body, Utc::now()) {
        return SharedAdapterError::RateLimited {
            detail,
            retry_at: throttle.retry_at,
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

#[cfg(test)]
mod tests {
    use super::*;

    use chrono::DateTime;

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
            ("x-ratelimit-remaining", "0"),
            ("x-ratelimit-reset", reset_epoch.as_str()),
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
    fn a_secondary_limit_is_rate_limited() {
        let map = headers(&[("retry-after", "90")]);
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
    }

    #[test]
    fn a_permission_refusal_is_access_denied() {
        let map = headers(&[("x-ratelimit-remaining", "4100")]);
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
