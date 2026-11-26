//! Random keys the installers write when an operator supplies none.

/// 32 random bytes, hex-encoded: the `TASK_TOKEN_SECRET` format.
pub(crate) fn hex_32() -> String {
    use std::fmt::Write as _;
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(&mut s, "{b:02x}");
    }
    s
}

/// 32 random bytes, base64-encoded: the `SECRETS_ENCRYPTION_KEY` format.
pub(crate) fn base64_32() -> String {
    use base64::Engine as _;
    let mut bytes = [0u8; 32];
    rand::fill(&mut bytes);
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
