//! argon2id hashing for local passwords.
//!
//! Hashing and verification are CPU-bound (~50-100ms). Callers on the request
//! path must go through the `spawn_blocking` wrappers here, never the raw
//! functions — an inline verify stalls a runtime worker and turns login into a
//! trivial DoS.

use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::RngExt;

/// argon2id parameters: m=19456 KiB, t=2, p=1 (OWASP baseline).
const MEMORY_KIB: u32 = 19_456;
const TIME_COST: u32 = 2;
const PARALLELISM: u32 = 1;

/// Alphabet for generated passwords. Indexed with a uniform `random_range`
/// — never `byte % 62`, which would bias the low end of the alphabet.
const PASSWORD_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

/// A PHC string produced with the parameters above, used to burn the same CPU
/// time as a real verify when the username is unknown.
const DUMMY_HASH: &str = "$argon2id$v=19$m=19456,t=2,p=1$c29tZXNhbHRzb21lc2FsdA$\
                          Iu6BbTFHUMHXKsPBvXNIkJXpDlmHNBAeZAiHl3Y9NNs";

#[derive(Debug, thiserror::Error)]
pub enum PasswordHashError {
    #[error("password hashing failed: {0}")]
    Hash(String),
    #[error("hashing task failed to run")]
    Join,
}

fn argon2() -> Result<Argon2<'static>, PasswordHashError> {
    let params = Params::new(MEMORY_KIB, TIME_COST, PARALLELISM, None)
        .map_err(|e| PasswordHashError::Hash(format!("invalid argon2 params: {e}")))?;
    Ok(Argon2::new(Algorithm::Argon2id, Version::V0x13, params))
}

/// Hash a password into a PHC string. Blocking — call via [`hash_password`].
fn hash_password_blocking(password: &str) -> Result<String, PasswordHashError> {
    // `SaltString::generate` wants argon2's own `rand_core 0.6` OsRng — the
    // backend's `rand 0.10` traits are unrelated types here.
    let salt = SaltString::generate(&mut OsRng);
    argon2()?
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| PasswordHashError::Hash(e.to_string()))
}

/// Verify a password against a PHC string. Blocking — call via [`verify_password`].
///
/// A malformed stored hash verifies as `false` rather than erroring, so a
/// corrupt row cannot be distinguished from a wrong password.
fn verify_password_blocking(password: &str, phc: &str) -> bool {
    let (Ok(parsed), Ok(argon2)) = (PasswordHash::new(phc), argon2()) else {
        tracing::warn!("stored password hash is not a usable argon2 PHC string");
        return false;
    };
    argon2.verify_password(password.as_bytes(), &parsed).is_ok()
}

/// Hash a password off the async runtime.
pub async fn hash_password(password: String) -> Result<String, PasswordHashError> {
    tokio::task::spawn_blocking(move || hash_password_blocking(&password))
        .await
        .map_err(|_| PasswordHashError::Join)?
}

/// Verify a password off the async runtime.
pub async fn verify_password(password: String, phc: String) -> Result<bool, PasswordHashError> {
    tokio::task::spawn_blocking(move || verify_password_blocking(&password, &phc))
        .await
        .map_err(|_| PasswordHashError::Join)
}

/// Burn a verify's worth of CPU against a fixed hash.
///
/// Called when the username is unknown so response timing does not leak
/// account existence.
pub async fn verify_dummy(password: String) {
    let _ =
        tokio::task::spawn_blocking(move || verify_password_blocking(&password, DUMMY_HASH)).await;
}

/// Generate a random password of `len` characters from [`PASSWORD_ALPHABET`].
pub fn generate_password(len: usize) -> String {
    let mut rng = rand::rng();
    (0..len)
        .map(|_| PASSWORD_ALPHABET[rng.random_range(0..PASSWORD_ALPHABET.len())] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hash_then_verify_round_trips() {
        let hash = hash_password("correct horse battery".into()).await.unwrap();
        assert!(hash.starts_with("$argon2id$"));
        assert!(
            verify_password("correct horse battery".into(), hash.clone())
                .await
                .unwrap()
        );
        assert!(!verify_password("wrong".into(), hash).await.unwrap());
    }

    #[tokio::test]
    async fn same_password_hashes_differently() {
        let a = hash_password("same".into()).await.unwrap();
        let b = hash_password("same".into()).await.unwrap();
        assert_ne!(a, b, "salt must differ between hashes");
    }

    #[tokio::test]
    async fn malformed_hash_verifies_false() {
        assert!(!verify_password("x".into(), "not-a-phc-string".into())
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn dummy_hash_is_valid_phc() {
        // If DUMMY_HASH stopped parsing, verify_dummy would return early and
        // the timing-equalisation it exists for would silently vanish.
        assert!(PasswordHash::new(DUMMY_HASH).is_ok());
        verify_dummy("anything".into()).await;
    }

    #[test]
    fn generated_passwords_are_alphabet_only_and_vary() {
        let a = generate_password(24);
        assert_eq!(a.chars().count(), 24);
        assert!(a.bytes().all(|b| PASSWORD_ALPHABET.contains(&b)));
        assert_ne!(a, generate_password(24));
    }
}
