//! GitHub webhook HMAC-SHA256 signature verification.

use moatless_adapter_model::error::{AdapterError, SharedAdapterError};
use moatless_adapter_model::signature::verify_hmac_sha256;

/// Verify the GitHub webhook signature.
///
/// GitHub signs the payload with `HMAC-SHA256(secret, body)` and sends the
/// result in the `X-Hub-Signature-256` header as `sha256=<hex>`.
pub(super) fn verify_signature(
    secret: &str,
    body: &[u8],
    signature_header: &str,
) -> Result<(), AdapterError> {
    let expected_hex = signature_header
        .strip_prefix("sha256=")
        .ok_or(SharedAdapterError::SignatureInvalid)?;
    verify_hmac_sha256(secret, body, expected_hex).map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    #[test]
    fn valid_signature() {
        let secret = "test-secret-123";
        let body = b"Hello, World!";

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
