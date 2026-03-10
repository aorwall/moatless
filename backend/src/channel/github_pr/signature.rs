//! GitHub webhook HMAC-SHA256 signature verification.

use hmac::{Hmac, Mac};
use sha2::Sha256;

use crate::channel::error::ChannelError;

type HmacSha256 = Hmac<Sha256>;

/// Verify the GitHub webhook signature.
///
/// GitHub signs every webhook payload with `HMAC-SHA256(secret, body)` and sends
/// the result in the `X-Hub-Signature-256` header as `sha256=<hex>`.
pub fn verify_signature(
    secret: &str,
    body: &[u8],
    signature_header: &str,
) -> Result<(), ChannelError> {
    let expected_hex = signature_header
        .strip_prefix("sha256=")
        .ok_or(ChannelError::SignatureInvalid)?;

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| ChannelError::Internal("Invalid webhook secret".to_string()))?;
    mac.update(body);

    let computed = hex::encode(mac.finalize().into_bytes());

    if computed != expected_hex {
        return Err(ChannelError::SignatureInvalid);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_signature() {
        let secret = "test-secret-123";
        let body = b"Hello, World!";

        // Compute expected signature
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
    fn missing_prefix() {
        let result = verify_signature("secret", b"body", "badbeef");
        assert!(result.is_err());
    }
}
