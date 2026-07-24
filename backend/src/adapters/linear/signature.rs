//! Linear webhook HMAC-SHA256 signature verification.
//!
//! Linear signs every webhook payload with `HMAC-SHA256(secret, raw_body)` and
//! sends the result as a hex string in the `Linear-Signature` header. Unlike
//! GitHub there is no `sha256=` prefix and no timestamp — replay protection
//! is provided by the `webhookId` dedup cache in the provider.

use crate::adapters::error::AdapterError;
use crate::adapters::signature::verify_hmac_sha256;

/// Verify the Linear webhook signature using a constant-time comparison.
pub fn verify_signature(
    secret: &str,
    body: &[u8],
    signature_header: &str,
) -> Result<(), AdapterError> {
    verify_hmac_sha256(secret, body, signature_header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hmac::{Hmac, KeyInit, Mac};
    use sha2::Sha256;

    type HmacSha256 = Hmac<Sha256>;

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
        mac.update(body);
        hex::encode(mac.finalize().into_bytes())
    }

    #[test]
    fn round_trip_verifies() {
        let secret = "linear-secret";
        let body = br#"{"action":"create"}"#;
        let sig = sign(secret, body);
        assert!(verify_signature(secret, body, &sig).is_ok());
    }

    #[test]
    fn mutated_body_rejected() {
        let secret = "linear-secret";
        let body = br#"{"action":"create"}"#;
        let sig = sign(secret, body);
        let tampered = br#"{"action":"update"}"#;
        assert!(verify_signature(secret, tampered, &sig).is_err());
    }

    #[test]
    fn empty_signature_rejected() {
        assert!(verify_signature("secret", b"body", "").is_err());
    }

    #[test]
    fn non_hex_signature_rejected() {
        assert!(verify_signature("secret", b"body", "not-hex!").is_err());
    }
}
