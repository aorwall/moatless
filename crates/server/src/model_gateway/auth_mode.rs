//! Restrictive credential selection requested by a model gateway caller.

use std::collections::HashMap;

use axum::http::HeaderMap;
use moatless_core::secret_keys::{ANTHROPIC_AUTH_TOKEN, CLAUDE_CODE_OAUTH_TOKEN, CODEX_AUTH_JSON};

/// Which existing credentials a caller's request may use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AuthMode {
    Default,
    ApiKeyOrHub,
}

impl AuthMode {
    /// Decode the optional restrictive policy, refusing ambiguous headers.
    pub(super) fn parse(headers: &HeaderMap) -> Result<Self, &'static str> {
        let mut values = headers.get_all("x-moatless-auth-mode").iter();
        match (values.next(), values.next()) {
            (None, None) => Ok(Self::Default),
            (Some(value), None) if value == "api-key-or-hub" => Ok(Self::ApiKeyOrHub),
            _ => Err("Unsupported X-Moatless-Auth-Mode"),
        }
    }

    /// Remove subscription and opaque bearer credentials for API-only callers.
    pub(super) fn filter(self, credentials: &mut HashMap<String, String>) {
        if self == Self::ApiKeyOrHub {
            for key in [
                ANTHROPIC_AUTH_TOKEN,
                CLAUDE_CODE_OAUTH_TOKEN,
                CODEX_AUTH_JSON,
            ] {
                credentials.remove(key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_policy_preserves_existing_credentials() {
        let mut credentials = HashMap::from([(CODEX_AUTH_JSON.into(), "login".into())]);
        AuthMode::parse(&HeaderMap::new())
            .unwrap()
            .filter(&mut credentials);
        assert_eq!(credentials[CODEX_AUTH_JSON], "login");
    }

    #[test]
    fn api_only_keeps_keys_and_endpoint_configuration() {
        let mut credentials = HashMap::from([
            ("OPENAI_API_KEY".into(), "key".into()),
            ("OPENAI_BASE_URL".into(), "https://hub/v1".into()),
            (CODEX_AUTH_JSON.into(), "login".into()),
            (CLAUDE_CODE_OAUTH_TOKEN.into(), "login".into()),
            (ANTHROPIC_AUTH_TOKEN.into(), "login".into()),
        ]);
        let mut headers = HeaderMap::new();
        headers.insert("x-moatless-auth-mode", "api-key-or-hub".parse().unwrap());
        AuthMode::parse(&headers).unwrap().filter(&mut credentials);
        assert_eq!(credentials.len(), 2);
        assert_eq!(credentials["OPENAI_API_KEY"], "key");
    }

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "test client sets 5s connect and 10s request timeouts; no request is sent"
    )]
    fn policy_header_does_not_reach_upstream() {
        let mut headers = HeaderMap::new();
        headers.insert("x-moatless-auth-mode", "api-key-or-hub".parse().unwrap());
        let request = super::super::stream::upstream_request(
            &reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(5))
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .unwrap(),
            axum::http::Method::POST,
            "https://example.com/v1/responses",
            &headers,
            axum::body::Bytes::new(),
        )
        .build()
        .unwrap();
        assert!(!request.headers().contains_key("x-moatless-auth-mode"));
    }

    #[test]
    fn unknown_or_duplicate_policy_is_refused() {
        let mut headers = HeaderMap::new();
        headers.insert("x-moatless-auth-mode", "unknown".parse().unwrap());
        assert!(AuthMode::parse(&headers).is_err());
        headers.insert("x-moatless-auth-mode", "api-key-or-hub".parse().unwrap());
        headers.append("x-moatless-auth-mode", "api-key-or-hub".parse().unwrap());
        assert!(AuthMode::parse(&headers).is_err());
    }
}
