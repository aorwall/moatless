//! Slack HMAC-SHA256 webhook signature verification.

use moatless_loops::adapters::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::signature::verify_hmac_sha256;

/// Verify the Slack request signature.
///
/// Slack signs every request with `v0=HMAC-SHA256(signing_secret, "v0:{timestamp}:{body}")`.
/// The signature is in the `X-Slack-Signature` header and the timestamp in
/// `X-Slack-Request-Timestamp`.
pub fn verify_signature(
    signing_secret: &str,
    timestamp: &str,
    body: &[u8],
    expected_signature: &str,
) -> Result<(), AdapterError> {
    // Reject requests older than 5 minutes to prevent replay attacks
    if let Ok(ts) = timestamp.parse::<i64>() {
        let now = chrono::Utc::now().timestamp();
        if (now - ts).unsigned_abs() > 300 {
            return Err(SharedAdapterError::SignatureInvalid.into());
        }
    }

    let expected_hex = expected_signature
        .strip_prefix("v0=")
        .ok_or(SharedAdapterError::SignatureInvalid)?;
    let sig_basestring = format!("v0:{timestamp}:{}", String::from_utf8_lossy(body));

    verify_hmac_sha256(signing_secret, sig_basestring.as_bytes(), expected_hex).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    #[test]
    fn valid_signature() {
        let secret = "8f742231b10e8888abcd99yez67676hg";
        let body = b"token=xyzz0WbapA4vBCDEFasx0q6G&team_id=T1DC2JH3J&api_app_id=A19GAJ72T";

        // Use a recent timestamp to bypass replay check
        let now = chrono::Utc::now().timestamp().to_string();
        let sig_basestring = format!("v0:{now}:{}", String::from_utf8_lossy(body));
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(sig_basestring.as_bytes());
        let expected = format!("v0={}", hex::encode(mac.finalize().into_bytes()));

        assert!(verify_signature(secret, &now, body, &expected).is_ok());
    }

    #[test]
    fn invalid_signature() {
        let now = chrono::Utc::now().timestamp().to_string();
        let result = verify_signature("secret", &now, b"body", "v0=badbeef");
        assert!(result.is_err());
    }

    #[test]
    fn stale_timestamp() {
        let old = (chrono::Utc::now().timestamp() - 600).to_string();
        let result = verify_signature("secret", &old, b"body", "v0=anything");
        assert!(result.is_err());
    }
}
