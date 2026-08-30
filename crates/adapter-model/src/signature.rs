//! Shared HMAC-SHA256 webhook signature verification.
//!
//! Every adapter authenticates inbound webhooks with `HMAC-SHA256(secret, payload)`
//! and differs only in how the expected digest is framed on the wire (a `v0=` or
//! `sha256=` prefix, or bare hex). The constant-time core lives here; each adapter
//! keeps a thin wrapper that strips its own framing before delegating.

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;

use crate::error::SharedAdapterError;

type HmacSha256 = Hmac<Sha256>;

/// Verify `expected_hex` is `HMAC-SHA256(secret, payload)`.
///
/// `expected_hex` is the bare hex digest (no prefix). Comparison is constant-time
/// via [`Mac::verify_slice`]; any decode or length mismatch is reported as
/// [`SharedAdapterError::SignatureInvalid`] so callers never leak which check failed.
pub fn verify_hmac_sha256(
    secret: &str,
    payload: &[u8],
    expected_hex: &str,
) -> Result<(), SharedAdapterError> {
    let expected = hex::decode(expected_hex).map_err(|_| SharedAdapterError::SignatureInvalid)?;

    let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
        .map_err(|_| SharedAdapterError::Internal("Invalid signing secret".to_string()))?;
    mac.update(payload);

    mac.verify_slice(&expected)
        .map_err(|_| SharedAdapterError::SignatureInvalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, payload: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("hmac key");
        mac.update(payload);
        hex::encode(mac.finalize().into_bytes())
    }

    #[test]
    fn round_trip_verifies() {
        let secret = "shared-secret";
        let payload = b"hello world";
        assert!(verify_hmac_sha256(secret, payload, &sign(secret, payload)).is_ok());
    }

    #[test]
    fn mutated_payload_rejected() {
        let secret = "shared-secret";
        let sig = sign(secret, b"hello world");
        assert!(verify_hmac_sha256(secret, b"HELLO WORLD", &sig).is_err());
    }

    #[test]
    fn non_hex_rejected() {
        assert!(verify_hmac_sha256("secret", b"body", "not-hex!").is_err());
    }

    #[test]
    fn empty_signature_rejected() {
        assert!(verify_hmac_sha256("secret", b"body", "").is_err());
    }
}
