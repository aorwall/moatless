//! Codex (OpenAI ChatGPT) OAuth 2.0 Device Code flow.
//!
//! Three round trips to `auth.openai.com`:
//!
//! 1. `POST /api/accounts/deviceauth/usercode` — issue a device_auth_id + user_code
//! 2. `POST /api/accounts/deviceauth/token`    — poll until the user has approved
//!    in their browser; returns an authorization code + PKCE verifier
//! 3. `POST /oauth/token` — exchange the authorization code for the final tokens
//!
//! On success we assemble Codex's `auth.json` shape so the existing sandbox
//! credential plumbing (`secrets.rs` → `init/credentials.rs` → `codex_watcher`)
//! works unchanged. `IdTokenClaims::parse` reads the (untrusted) JWT body so the
//! UI can show the connected account's email + plan.

use base64::Engine;
use chrono::Utc;
use reqwest::StatusCode;
use serde::Deserialize;
use tracing::warn;

const REDIRECT_URI: &str = "https://auth.openai.com/deviceauth/callback";
const VERIFICATION_URL: &str = "https://auth.openai.com/codex/device";
const FALLBACK_INTERVAL_SECS: u64 = 5;
const EXPIRES_IN_SECS: u64 = 900;

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

/// Subset of claims we care about from the OAuth id_token JWT.
///
/// The JWT signature is NOT verified — we only read it for display. The
/// trustworthy step is the server-side token exchange against `oauth/token`
/// which runs over TLS to `auth.openai.com` and only accepts `code_verifier`
/// pairs we received over that same channel.
#[derive(Debug, Clone, Default)]
pub struct IdTokenClaims {
    pub email: Option<String>,
    pub chatgpt_plan_type: Option<String>,
    pub chatgpt_account_id: Option<String>,
    pub chatgpt_user_id: Option<String>,
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

/// Step 1: ask `auth.openai.com` for a device_auth_id + user_code.
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

/// Step 2 + 3: one poll iteration. The frontend drives the cadence — we just
/// do a single token poll, and on success immediately exchange for the final
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
/// Codex CLI does this immediately after the initial code exchange — without
/// it, `auth.json` arrives with `OPENAI_API_KEY: null` and Codex falls back to
/// a refresh-on-startup using the bearer tokens. Two parallel subprocesses
/// then race to consume the (rotating) refresh token, and the loser gets
/// `refresh_token_reused` and the session dies.
///
/// Does NOT consume the refresh_token — only `id_token` is sent as the
/// subject_token. Failure here is non-fatal upstream; we mirror that and let
/// the caller record `null` if this fails.
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

impl IdTokenClaims {
    /// Decode the JWT body without verifying the signature. Claims under the
    /// `https://api.openai.com/auth` namespace carry the ChatGPT account info.
    pub fn parse(jwt: &str) -> Result<Self, String> {
        let mut parts = jwt.split('.');
        let _header = parts.next().ok_or("missing JWT header")?;
        let payload_b64 = parts.next().ok_or("missing JWT payload")?;

        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(payload_b64)
            .map_err(|e| format!("base64 decode: {e}"))?;
        let payload: serde_json::Value =
            serde_json::from_slice(&decoded).map_err(|e| format!("json parse: {e}"))?;

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
        assert_eq!(parsed.device_auth_id, "deviceauth_69f4420b5fd88191911f124c85b83ef1");
        assert_eq!(parsed.user_code, "CGSR-90ECR");
        assert_eq!(parse_loose_u64(parsed.interval.as_ref().unwrap()), Some(5));
        assert!(duration_until_rfc3339(parsed.expires_at.as_deref().unwrap()).unwrap() > 0);
    }
}
