//! Codex (OpenAI ChatGPT) OAuth 2.0 Device Code flow.
//!
//! Three round trips to `auth.openai.com`: issue a `device_auth_id` +
//! `user_code`, poll until the user approves in-browser for an authorization
//! code + PKCE verifier, then exchange that code for the final tokens.
//!
//! On success the result is assembled into Codex's `auth.json` shape so the
//! existing sandbox credential plumbing works unchanged.

use base64::Engine;
use chrono::{DateTime, Utc};
use reqwest::StatusCode;
use serde::Deserialize;
use tracing::warn;

const REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
const FALLBACK_INTERVAL_SECS: u64 = 5;
const EXPIRES_IN_SECS: u64 = 900;

/// OpenAI's first-party Codex CLI `client_id`. Hardcoded to match the upstream
/// Codex CLI; if OpenAI rotates it, every refresh will start failing and we'll
/// need to bump this constant.
pub const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

const REFRESH_URL_DEFAULT: &str = "https://auth.openai.com/oauth/token";
const REFRESH_URL_OVERRIDE_ENV: &str = "CODEX_REFRESH_TOKEN_URL_OVERRIDE";

/// Recognized OAuth error codes that indicate the refresh token will never
/// succeed again — caller must surface a "reconnect Codex" message.
const ERROR_CODE_EXPIRED: &str = "refresh_token_expired";
const ERROR_CODE_EXHAUSTED: &str = "refresh_token_reused";
const ERROR_CODE_REVOKED: &str = "refresh_token_invalidated";

#[derive(Debug, thiserror::Error)]
pub enum CodexOAuthError {
    #[error("Network error talking to Codex OAuth: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Codex OAuth returned {status}: {body}")]
    RemoteStatus { status: StatusCode, body: String },

    #[error("Failed to decode Codex OAuth response: {0}")]
    Decode(String),
}

#[derive(Debug, Clone)]
pub struct DeviceCodeStart {
    pub device_auth_id: String,
    pub user_code: String,
    pub verification_url: String,
    pub interval_secs: u64,
    pub expires_in_secs: u64,
}

#[derive(Debug)]
pub enum PollStatus {
    Pending,
    Success {
        auth_json: serde_json::Value,
        id_token_claims: IdTokenClaims,
    },
    Expired,
}

/// Subset of claims we care about from the OAuth `id_token` JWT.
///
/// The signature is NOT verified — only the server-side exchange against
/// `oauth/token` (TLS, matched `code_verifier`) is trustworthy.
#[derive(Debug, Clone, Default)]
pub struct IdTokenClaims {
    pub email: Option<String>,
    pub chatgpt_plan_type: Option<String>,
    pub chatgpt_account_id: Option<String>,
    pub chatgpt_user_id: Option<String>,
}

/// Result of a successful `refresh_token` grant — any subset of the three
/// tokens may be present (upstream only returns rotated values).
#[derive(Debug, Clone, Default)]
pub struct RefreshedTokens {
    pub id_token: Option<String>,
    pub access_token: Option<String>,
    pub refresh_token: Option<String>,
}

/// Why a refresh failed permanently. Caller writes this to
/// `provider_tokens.metadata.refresh_failure` so the UI can surface a
/// "reconnect Codex" prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermanentReason {
    /// Refresh token TTL elapsed — issued long ago, never used again.
    Expired,
    /// Refresh token was already used once (rotation race).
    Exhausted,
    /// Token was explicitly revoked (manual disconnect upstream).
    Revoked,
    /// 401 without a known error code.
    Unauthorized,
    /// Other recognized permanent failure.
    Other,
}

/// Errors returned by [`refresh_chatgpt_tokens`].
#[derive(Debug, thiserror::Error)]
pub enum RefreshTokenError {
    /// Will never succeed without user re-auth. Caller stops retrying and
    /// records the failure on the `provider_tokens` row.
    #[error("permanent: {reason:?} ({body})")]
    Permanent {
        reason: PermanentReason,
        body: String,
    },
    /// Transient error — retry later. Caller can return the stale blob and let
    /// the sandbox CLI take another shot.
    #[error("transient ({status}): {body}")]
    Transient { status: StatusCode, body: String },
    /// Network / connection failure.
    #[error(transparent)]
    Network(#[from] reqwest::Error),
    /// Refresh succeeded but the response body did not parse.
    #[error("decode failure: {0}")]
    Decode(String),
}

#[derive(Deserialize)]
#[allow(clippy::struct_field_names)] // reason: matches OAuth wire format
struct RefreshResponse {
    #[serde(default)]
    id_token: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
}

fn refresh_endpoint() -> String {
    std::env::var(REFRESH_URL_OVERRIDE_ENV).unwrap_or_else(|_| REFRESH_URL_DEFAULT.to_string())
}

/// Extract a normalized lowercase OAuth error code from a response body.
///
/// Checks `error.code`, then `error`, then top-level `code`, matching upstream
/// Codex CLI order (`manager.rs:860-913`).
fn extract_refresh_token_error_code(body: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(body).ok()?;

    if let Some(code) = parsed
        .get("error")
        .and_then(|err| err.get("code"))
        .and_then(serde_json::Value::as_str)
    {
        return Some(code.to_ascii_lowercase());
    }
    if let Some(code) = parsed.get("error").and_then(serde_json::Value::as_str) {
        return Some(code.to_ascii_lowercase());
    }
    if let Some(code) = parsed.get("code").and_then(serde_json::Value::as_str) {
        return Some(code.to_ascii_lowercase());
    }
    None
}

/// Classify a non-2xx refresh response.
fn classify_refresh_error(status: StatusCode, body: String) -> RefreshTokenError {
    let code = extract_refresh_token_error_code(&body);

    if let Some(ref code) = code {
        let reason = match code.as_str() {
            ERROR_CODE_EXPIRED => Some(PermanentReason::Expired),
            ERROR_CODE_EXHAUSTED => Some(PermanentReason::Exhausted),
            ERROR_CODE_REVOKED => Some(PermanentReason::Revoked),
            _ => None,
        };
        if let Some(reason) = reason {
            return RefreshTokenError::Permanent { reason, body };
        }
    }

    if status == StatusCode::UNAUTHORIZED {
        let reason = if code.is_some() {
            PermanentReason::Other
        } else {
            PermanentReason::Unauthorized
        };
        return RefreshTokenError::Permanent { reason, body };
    }

    RefreshTokenError::Transient { status, body }
}

/// POST a refresh-token grant to OpenAI's OAuth token endpoint and return the
/// rotated tokens.
///
/// A successful refresh rotates the `refresh_token` server-side and the old one
/// is dead immediately, so callers must persist the new one before anything
/// else can race them.
pub async fn refresh_chatgpt_tokens(
    client: &reqwest::Client,
    refresh_token: &str,
) -> Result<RefreshedTokens, RefreshTokenError> {
    let url = refresh_endpoint();
    let body = serde_json::json!({
        "client_id": CODEX_CLIENT_ID,
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
    });

    let resp = client
        .post(&url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&body)
        .send()
        .await?;
    let status = resp.status();

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(classify_refresh_error(status, body));
    }

    let parsed: RefreshResponse = resp
        .json()
        .await
        .map_err(|e| RefreshTokenError::Decode(format!("refresh response: {e}")))?;

    Ok(RefreshedTokens {
        id_token: parsed.id_token,
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token,
    })
}

#[derive(Deserialize)]
struct UserCodeResponse {
    device_auth_id: String,
    user_code: String,
    /// `auth.openai.com` returns this as a JSON string ("5"), not a number, so
    /// keep it loose and parse below.
    #[serde(default)]
    interval: Option<serde_json::Value>,
    #[serde(default)]
    expires_at: Option<String>,
}

fn parse_loose_u64(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => s.parse::<u64>().ok(),
        _ => None,
    }
}

fn duration_until_rfc3339(value: &str) -> Option<u64> {
    let target = chrono::DateTime::parse_from_rfc3339(value).ok()?;
    let remaining = target.timestamp() - chrono::Utc::now().timestamp();
    u64::try_from(remaining.max(0)).ok()
}

#[derive(Deserialize)]
struct PollTokenResponse {
    authorization_code: String,
    code_verifier: String,
}

#[derive(Deserialize)]
#[allow(clippy::struct_field_names)] // reason: matches OAuth wire format
struct OAuthTokenResponse {
    id_token: String,
    access_token: String,
    refresh_token: String,
}

#[derive(Deserialize)]
struct TokenExchangeResponse {
    access_token: String,
}

/// Step 1: ask `auth.openai.com` for a `device_auth_id` + `user_code`.
pub async fn request_device_code(
    client: &reqwest::Client,
    issuer: &str,
    client_id: &str,
) -> Result<DeviceCodeStart, CodexOAuthError> {
    let url = format!("{issuer}/api/accounts/deviceauth/usercode");
    let body = serde_json::json!({ "client_id": client_id });
    let resp = client.post(&url).json(&body).send().await?;
    let status = resp.status();

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(CodexOAuthError::RemoteStatus { status, body });
    }

    let parsed: UserCodeResponse = resp
        .json()
        .await
        .map_err(|e| CodexOAuthError::Decode(format!("usercode response: {e}")))?;

    let interval_secs = parsed
        .interval
        .as_ref()
        .and_then(parse_loose_u64)
        .unwrap_or(FALLBACK_INTERVAL_SECS);
    let expires_in_secs = parsed
        .expires_at
        .as_deref()
        .and_then(duration_until_rfc3339)
        .filter(|s| *s > 0)
        .unwrap_or(EXPIRES_IN_SECS);

    Ok(DeviceCodeStart {
        device_auth_id: parsed.device_auth_id,
        user_code: parsed.user_code,
        verification_url: VERIFICATION_URL.to_string(),
        interval_secs,
        expires_in_secs,
    })
}

/// Step 2 + 3: one poll iteration. The frontend drives the cadence — we just do
/// a single token poll, and on success immediately exchange for the final
/// tokens and assemble the auth.json blob.
pub async fn poll_once(
    client: &reqwest::Client,
    issuer: &str,
    client_id: &str,
    device_auth_id: &str,
    user_code: &str,
) -> Result<PollStatus, CodexOAuthError> {
    let url = format!("{issuer}/api/accounts/deviceauth/token");
    let body = serde_json::json!({
        "device_auth_id": device_auth_id,
        "user_code": user_code,
    });
    let resp = client.post(&url).json(&body).send().await?;
    let status = resp.status();

    // 403/404 = still pending. 410 (or matching expired text) = code expired.
    if status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND {
        return Ok(PollStatus::Pending);
    }
    if status == StatusCode::GONE {
        return Ok(PollStatus::Expired);
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        if body.contains("expired") {
            return Ok(PollStatus::Expired);
        }
        return Err(CodexOAuthError::RemoteStatus { status, body });
    }

    let parsed: PollTokenResponse = resp
        .json()
        .await
        .map_err(|e| CodexOAuthError::Decode(format!("poll response: {e}")))?;

    let tokens = exchange_code_for_tokens(
        client,
        issuer,
        client_id,
        &parsed.authorization_code,
        &parsed.code_verifier,
    )
    .await?;

    let claims = IdTokenClaims::parse(&tokens.id_token).unwrap_or_else(|e| {
        warn!(error = %e, "Could not parse Codex id_token claims; continuing without metadata");
        IdTokenClaims::default()
    });

    // Mint OPENAI_API_KEY via token-exchange — must happen here, otherwise the
    // sandbox-side Codex CLI will try to refresh on startup and crash with
    // refresh_token_reused once two of its subprocesses race.
    let openai_api_key = match obtain_api_key(client, issuer, client_id, &tokens.id_token).await {
        Ok(key) => Some(key),
        Err(e) => {
            warn!(error = %e, "Codex token-exchange for OPENAI_API_KEY failed (non-fatal)");
            None
        }
    };

    let auth_json = serde_json::json!({
        "auth_mode": "chatgpt",
        "OPENAI_API_KEY": openai_api_key,
        "tokens": {
            "id_token": tokens.id_token,
            "access_token": tokens.access_token,
            "refresh_token": tokens.refresh_token,
            "account_id": claims.chatgpt_account_id.clone().unwrap_or_default(),
        },
        "last_refresh": Utc::now().to_rfc3339(),
    });

    Ok(PollStatus::Success {
        auth_json,
        id_token_claims: claims,
    })
}

async fn exchange_code_for_tokens(
    client: &reqwest::Client,
    issuer: &str,
    client_id: &str,
    code: &str,
    code_verifier: &str,
) -> Result<OAuthTokenResponse, CodexOAuthError> {
    let url = format!("{issuer}/oauth/token");
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", REDIRECT_URI),
        ("client_id", client_id),
        ("code_verifier", code_verifier),
    ];

    let resp = client.post(&url).form(&form).send().await?;
    let status = resp.status();

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(CodexOAuthError::RemoteStatus { status, body });
    }

    resp.json::<OAuthTokenResponse>()
        .await
        .map_err(|e| CodexOAuthError::Decode(format!("token exchange response: {e}")))
}

/// Mint an `OPENAI_API_KEY` (sk-...) via the OAuth token-exchange grant.
///
/// Skipping this leaves `OPENAI_API_KEY: null`, forcing parallel subprocesses
/// to race the rotating refresh token; does NOT consume `refresh_token` (only
/// `id_token` is sent), and failure here is non-fatal — the caller records
/// `null`.
async fn obtain_api_key(
    client: &reqwest::Client,
    issuer: &str,
    client_id: &str,
    id_token: &str,
) -> Result<String, CodexOAuthError> {
    let url = format!("{issuer}/oauth/token");
    let form = [
        (
            "grant_type",
            "urn:ietf:params:oauth:grant-type:token-exchange",
        ),
        ("client_id", client_id),
        ("requested_token", "openai-api-key"),
        ("subject_token", id_token),
        (
            "subject_token_type",
            "urn:ietf:params:oauth:token-type:id_token",
        ),
    ];

    let resp = client.post(&url).form(&form).send().await?;
    let status = resp.status();

    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(CodexOAuthError::RemoteStatus { status, body });
    }

    let parsed: TokenExchangeResponse = resp
        .json()
        .await
        .map_err(|e| CodexOAuthError::Decode(format!("api-key exchange response: {e}")))?;

    Ok(parsed.access_token)
}

/// Decode the body (second segment) of a JWT without verifying the signature.
/// Shared by [`IdTokenClaims::parse`] and [`parse_jwt_exp`].
fn decode_jwt_payload(jwt: &str) -> Result<serde_json::Value, String> {
    let mut parts = jwt.split('.');
    let _header = parts.next().ok_or("missing JWT header")?;
    let payload_b64 = parts.next().ok_or("missing JWT payload")?;

    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .map_err(|e| format!("base64 decode: {e}"))?;
    serde_json::from_slice(&decoded).map_err(|e| format!("json parse: {e}"))
}

/// Extract the `exp` claim from a JWT as a UTC timestamp. Returns `Ok(None)`
/// when the claim is absent or not a number; returns `Err` only when the JWT
/// cannot be decoded at all.
pub fn parse_jwt_exp(jwt: &str) -> Result<Option<DateTime<Utc>>, String> {
    let payload = decode_jwt_payload(jwt)?;
    let exp = payload.get("exp").and_then(serde_json::Value::as_i64);
    Ok(exp.and_then(|secs| DateTime::<Utc>::from_timestamp(secs, 0)))
}

impl IdTokenClaims {
    /// Decode the JWT body without verifying the signature. Claims under the
    /// `https://api.openai.com/auth` namespace carry the ChatGPT account info.
    pub fn parse(jwt: &str) -> Result<Self, String> {
        let payload = decode_jwt_payload(jwt)?;

        let email = payload
            .get("email")
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);

        let auth_ns = payload.get("https://api.openai.com/auth");
        let chatgpt_plan_type = auth_ns
            .and_then(|v| v.get("chatgpt_plan_type"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);
        let chatgpt_account_id = auth_ns
            .and_then(|v| v.get("chatgpt_account_id"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);
        let chatgpt_user_id = auth_ns
            .and_then(|v| v.get("chatgpt_user_id"))
            .and_then(|v| v.as_str())
            .map(std::string::ToString::to_string);

        Ok(Self {
            email,
            chatgpt_plan_type,
            chatgpt_account_id,
            chatgpt_user_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_jwt(payload: &serde_json::Value) -> String {
        let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"{\"alg\":\"none\"}");
        let body = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(payload).expect("encode"));
        format!("{header}.{body}.signaturepart")
    }

    #[test]
    fn parses_email_and_chatgpt_claims() {
        let payload = serde_json::json!({
            "email": "user@example.com",
            "https://api.openai.com/auth": {
                "chatgpt_plan_type": "plus",
                "chatgpt_account_id": "acc_123",
                "chatgpt_user_id": "user_456",
            }
        });
        let jwt = make_jwt(&payload);
        let claims = IdTokenClaims::parse(&jwt).expect("parse");
        assert_eq!(claims.email.as_deref(), Some("user@example.com"));
        assert_eq!(claims.chatgpt_plan_type.as_deref(), Some("plus"));
        assert_eq!(claims.chatgpt_account_id.as_deref(), Some("acc_123"));
        assert_eq!(claims.chatgpt_user_id.as_deref(), Some("user_456"));
    }

    #[test]
    fn parse_returns_default_when_namespace_missing() {
        let payload = serde_json::json!({ "email": "user@example.com" });
        let jwt = make_jwt(&payload);
        let claims = IdTokenClaims::parse(&jwt).expect("parse");
        assert_eq!(claims.email.as_deref(), Some("user@example.com"));
        assert!(claims.chatgpt_plan_type.is_none());
        assert!(claims.chatgpt_account_id.is_none());
    }

    #[test]
    fn parse_rejects_malformed_jwt() {
        assert!(IdTokenClaims::parse("not-a-jwt").is_err());
    }

    #[test]
    fn parse_loose_u64_accepts_string_and_number() {
        assert_eq!(
            parse_loose_u64(&serde_json::json!(5)),
            Some(5),
            "JSON number should parse"
        );
        assert_eq!(
            parse_loose_u64(&serde_json::json!("5")),
            Some(5),
            "JSON string should parse"
        );
        assert_eq!(
            parse_loose_u64(&serde_json::json!("not a number")),
            None,
            "non-numeric string should yield None"
        );
        assert_eq!(parse_loose_u64(&serde_json::json!(null)), None);
    }

    #[test]
    fn user_code_response_decodes_real_openai_payload() {
        // Captured from a live `auth.openai.com` response on 2026-05-01.
        let body = serde_json::json!({
            "device_auth_id": "deviceauth_69f4420b5fd88191911f124c85b83ef1",
            "user_code": "CGSR-90ECR",
            "interval": "5",
            "expires_at": "2099-01-01T00:00:00+00:00"
        });
        let parsed: UserCodeResponse = serde_json::from_value(body).expect("decode");
        assert_eq!(
            parsed.device_auth_id,
            "deviceauth_69f4420b5fd88191911f124c85b83ef1"
        );
        assert_eq!(parsed.user_code, "CGSR-90ECR");
        assert_eq!(parse_loose_u64(parsed.interval.as_ref().unwrap()), Some(5));
        assert!(duration_until_rfc3339(parsed.expires_at.as_deref().unwrap()).unwrap() > 0);
    }

    // =========================================================================
    // JWT `exp` parsing (Slice B)
    // =========================================================================

    #[test]
    fn parse_jwt_exp_returns_timestamp() {
        let exp_secs: i64 = 1_900_000_000; // some future timestamp
        let payload = serde_json::json!({ "exp": exp_secs });
        let jwt = make_jwt(&payload);
        let got = parse_jwt_exp(&jwt).expect("parse").expect("exp present");
        assert_eq!(got.timestamp(), exp_secs);
    }

    #[test]
    fn parse_jwt_exp_returns_none_when_absent() {
        let payload = serde_json::json!({ "email": "user@example.com" });
        let jwt = make_jwt(&payload);
        let got = parse_jwt_exp(&jwt).expect("parse");
        assert!(got.is_none(), "missing exp claim should yield None");
    }

    #[test]
    fn parse_jwt_exp_errors_on_malformed_jwt() {
        assert!(parse_jwt_exp("not-a-jwt").is_err());
    }

    // =========================================================================
    // Refresh error classification (Slice A)
    // =========================================================================

    #[test]
    fn classify_refresh_error_extracts_code_from_object_form() {
        let body = serde_json::json!({ "error": { "code": "refresh_token_reused" } }).to_string();
        let err = classify_refresh_error(StatusCode::BAD_REQUEST, body);
        match err {
            RefreshTokenError::Permanent { reason, .. } => {
                assert_eq!(reason, PermanentReason::Exhausted);
            }
            other => panic!("expected Permanent(Exhausted), got {other:?}"),
        }
    }

    #[test]
    fn classify_refresh_error_extracts_code_from_string_form() {
        let body = serde_json::json!({ "error": "refresh_token_expired" }).to_string();
        let err = classify_refresh_error(StatusCode::BAD_REQUEST, body);
        match err {
            RefreshTokenError::Permanent { reason, .. } => {
                assert_eq!(reason, PermanentReason::Expired);
            }
            other => panic!("expected Permanent(Expired), got {other:?}"),
        }
    }

    #[test]
    fn classify_refresh_error_extracts_code_from_top_level() {
        let body = serde_json::json!({ "code": "refresh_token_invalidated" }).to_string();
        let err = classify_refresh_error(StatusCode::BAD_REQUEST, body);
        match err {
            RefreshTokenError::Permanent { reason, .. } => {
                assert_eq!(reason, PermanentReason::Revoked);
            }
            other => panic!("expected Permanent(Revoked), got {other:?}"),
        }
    }

    #[test]
    fn classify_refresh_error_treats_401_without_code_as_unauthorized() {
        let err = classify_refresh_error(StatusCode::UNAUTHORIZED, "not json".into());
        match err {
            RefreshTokenError::Permanent { reason, .. } => {
                assert_eq!(reason, PermanentReason::Unauthorized);
            }
            other => panic!("expected Permanent(Unauthorized), got {other:?}"),
        }
    }

    #[test]
    fn classify_refresh_error_classifies_5xx_as_transient() {
        let err = classify_refresh_error(StatusCode::BAD_GATEWAY, "upstream broke".into());
        match err {
            RefreshTokenError::Transient { status, .. } => {
                assert_eq!(status, StatusCode::BAD_GATEWAY);
            }
            other => panic!("expected Transient, got {other:?}"),
        }
    }

    #[test]
    fn classify_refresh_error_unrecognized_code_is_transient_when_not_401() {
        let body = serde_json::json!({ "error": "something_else" }).to_string();
        let err = classify_refresh_error(StatusCode::BAD_REQUEST, body);
        assert!(
            matches!(err, RefreshTokenError::Transient { .. }),
            "unknown error code at 400 should be Transient, got {err:?}"
        );
    }
}
