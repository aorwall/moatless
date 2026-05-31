//! Webhook HMAC-SHA256 signature verification.
//!
//! Verifies `X-Webhook-Signature` header using `HMAC-SHA256(secret, body)`.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::adapters::error::AdapterError;

type HmacSha256 = Hmac<Sha256>;

/// Verify the webhook request signature.
///
/// Expected signature format: `sha256=<hex-encoded HMAC>`.
/// The HMAC is computed as `HMAC-SHA256(secret, body)`.
pub fn verify_signature(
    secret: &str,
    body: &[u8],
    expected_signature: &str,
) -> Result<(), AdapterError> {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| AdapterError::Internal("Invalid webhook secret".to_string()))?;
    mac.update(body);
    let computed = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

    if computed != expected_signature {
        return Err(AdapterError::SignatureInvalid);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_signature() {
        let secret = "my-webhook-secret";
        let body = b"hello world";

        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let expected = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));

        assert!(verify_signature(secret, body, &expected).is_ok());
    }

    #[test]
    fn invalid_signature() {
        let result = verify_signature("secret", b"body", "sha256=badbeef");
        assert!(result.is_err());
    }

    #[test]
    fn no_prefix_fails() {
        let result = verify_signature("secret", b"body", "badbeef");
        assert!(result.is_err());
    }
}
