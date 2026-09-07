//! The password to a task's workspace `restic` repository.
//!
//! One master secret per deployment, one password per task, derived from it.
//! The password reaches the sandbox pod, and a pod is handed only its own
//! task's, so a password that escapes one pod opens that task's repository and
//! no other.

use hkdf::Hkdf;
use sha2::Sha256;

/// Domain separator, so this master secret cannot be made to produce a key any
/// other derivation in this deployment also produces.
const INFO_PREFIX: &[u8] = b"moatless/workspace-repository/v1/";

/// The repository password for `task_id`.
///
/// Deterministic: the same master secret and task id always give the same
/// password, which is what lets any backend replica open a repository another
/// replica created.
pub fn derive_repository_password(master_secret: &str, task_id: &str) -> String {
    let hkdf = Hkdf::<Sha256>::new(None, master_secret.as_bytes());
    let mut key = [0u8; 32];
    let mut info = INFO_PREFIX.to_vec();
    info.extend_from_slice(task_id.as_bytes());
    #[expect(
        clippy::expect_used,
        reason = "the only error is an output longer than 255 hash blocks, and 32 bytes is one"
    )]
    hkdf.expand(&info, &mut key)
        .expect("32 bytes is one HKDF block");
    key.iter().fold(String::with_capacity(64), |mut hex, byte| {
        use std::fmt::Write;
        // `write!` to a `String` cannot fail.
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_is_stable_and_differs_per_task() {
        let first = derive_repository_password("master", "task-a");
        assert_eq!(first, derive_repository_password("master", "task-a"));
        assert_ne!(first, derive_repository_password("master", "task-b"));
        assert_ne!(first, derive_repository_password("other", "task-a"));
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
    }

    /// A task id that is a prefix of another must not share its password.
    #[test]
    fn a_task_id_prefix_gets_its_own_password() {
        assert_ne!(
            derive_repository_password("master", "task"),
            derive_repository_password("master", "task-1")
        );
    }
}
