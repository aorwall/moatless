use super::models::PairingRequest;
use moatless_adapter_model::error::AdapterError;
use moatless_core::db::DbPool;
use rand::RngExt;
use tracing::info;

/// Alphabet for pairing codes — no O/I/0/1 to avoid confusion.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const CODE_LENGTH: usize = 8;

/// Generate a random pairing code.
pub fn generate_code() -> String {
    let mut rng = rand::rng();
    (0..CODE_LENGTH)
        .map(|_| CODE_ALPHABET[rng.random_range(0..CODE_ALPHABET.len())] as char)
        .collect()
}

/// Upsert a pending pairing request. Returns existing code if already pending.
pub async fn upsert_pairing_request(
    db: &DbPool,
    loop_id: &str,
    provider_user_id: &str,
    meta: Option<serde_json::Value>,
) -> Result<PairingRequest, AdapterError> {
    let existing = super::dao::find_pending(db, loop_id, provider_user_id).await?;
    if let Some(req) = existing {
        return Ok(req);
    }

    let code = generate_code();

    let row = super::dao::insert_pending(db, loop_id, provider_user_id, &code, meta).await?;
    info!(
        loop_id,
        provider_user_id,
        code = %row.code,
        "Created pairing request"
    );

    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_generation_format() {
        let code = generate_code();
        assert_eq!(code.len(), CODE_LENGTH);
        for ch in code.chars() {
            assert!(
                CODE_ALPHABET.contains(&(ch as u8)),
                "Code contains invalid char: {ch}"
            );
        }
    }

    #[test]
    fn code_excludes_ambiguous_chars() {
        // Verify no O, I, 0, 1 in alphabet
        assert!(!CODE_ALPHABET.contains(&b'O'));
        assert!(!CODE_ALPHABET.contains(&b'I'));
        assert!(!CODE_ALPHABET.contains(&b'0'));
        assert!(!CODE_ALPHABET.contains(&b'1'));
    }

    #[test]
    fn codes_are_unique() {
        let codes: Vec<String> = (0..100).map(|_| generate_code()).collect();
        let unique: std::collections::HashSet<&String> = codes.iter().collect();
        // With 8 chars from 30-char alphabet, collisions in 100 samples are astronomically unlikely
        assert_eq!(codes.len(), unique.len());
    }
}
