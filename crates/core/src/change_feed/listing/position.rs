//! The durable record that a Task's listing-visible state changed.
//!
//! One counter row for the whole deployment, held until commit. See the module
//! doc on [`super::super`] for why the hold is what makes commit order and
//! position order the same order, and for this counter's place in the lock
//! order.
//!
//! No T3 vocabulary appears here — `moatless-t3code` turns the record into a
//! shell event.

use sha2::{Digest, Sha256};

/// How far behind a client may be and still be replayed rather than rebuilt.
/// Positions rather than an age, because positions are dense and rows are the
/// cost; 5,000 is ten `CHANGES_PER_POLL` batches.
pub const RESUME_WINDOW: i64 = 5_000;

/// How many of the most recent changes are kept.
///
/// Must stay wider than [`RESUME_WINDOW`]: [`resume`] decides off the head and
/// the catch-up poll reads moments later, so equal numbers let a prune delete
/// the record just admitted.
pub const RETAINED_CHANGES: i64 = 2 * RESUME_WINDOW;

/// At compile time rather than in a test, because the failure it prevents is
/// silent. See [`RETAINED_CHANGES`].
const _: () = assert!(RETAINED_CHANGES > RESUME_WINDOW);

/// Which database this deployment's positions were allocated in: the cluster's
/// `system_identifier` and the database's OID, joined.
///
/// Neither half is a row anything writes — an identity stored *in* the database
/// would be in the dump, so a restore would report the copy as the original.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentIdentity(String);

impl EnvironmentIdentity {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// A short, opaque name changing exactly when the identity does. The
    /// identity is operational detail, and the descriptor carrying it is
    /// public.
    pub fn digest(&self) -> String {
        let digest = Sha256::digest(self.0.as_bytes());
        digest[..8].iter().fold(String::new(), |mut hex, byte| {
            use std::fmt::Write;
            let _ = write!(hex, "{byte:02x}");
            hex
        })
    }
}

/// The position this deployment has reached, and the database it reached it in.
///
/// One read, deliberately: read apart, a database replaced between them yields
/// one deployment's position carrying the other's identity — the state
/// [`resume`] refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListingHead {
    pub identity: EnvironmentIdentity,
    pub position: i64,
}

/// Where a resuming client is put. The two look alike unless named apart, and a
/// client that cannot tell them apart renders a stale sidebar while reporting
/// itself live.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    /// Everything after this position is still recorded. Replay from it.
    From(i64),
    /// Further behind than [`RESUME_WINDOW`], or ahead of a position this
    /// deployment has ever reached. Build the listing again.
    Rebuild,
}

/// Decide whether a client holding `after` can be replayed.
///
/// `deployment` is the identity read at startup; `database`/`head` come from
/// [`current_head`](super::dao::current_head), now. Rebuilds on: a mismatched
/// database, a position never reached, or a cursor further behind than
/// [`RESUME_WINDOW`].
pub fn resume(
    deployment: &EnvironmentIdentity,
    database: &EnvironmentIdentity,
    head: i64,
    after: i64,
) -> Resume {
    if database != deployment {
        return Resume::Rebuild;
    }
    if after > head {
        return Resume::Rebuild;
    }
    if head - after > RESUME_WINDOW {
        return Resume::Rebuild;
    }
    Resume::From(after)
}

/// One recorded change: what moved, and where it sits relative to every other.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ListingChange {
    pub position: i64,
    pub task_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ours() -> EnvironmentIdentity {
        EnvironmentIdentity::new("7667539899605483559:16386")
    }

    #[test]
    fn a_cursor_is_replayable_up_to_the_resume_window_and_no_further() {
        let resume = |head, after| resume(&ours(), &ours(), head, after);
        assert_eq!(resume(0, 0), Resume::From(0));
        assert_eq!(resume(10, 10), Resume::From(10));
        assert_eq!(resume(10, 9), Resume::From(9));
        // Exactly the window, and one past it.
        assert_eq!(resume(RESUME_WINDOW, 0), Resume::From(0));
        assert_eq!(resume(RESUME_WINDOW + 1, 0), Resume::Rebuild);
        // Ahead of anything this deployment has reached.
        assert_eq!(resume(0, 1), Resume::Rebuild);
        assert_eq!(resume(10, 11), Resume::Rebuild);
    }

    /// A restore's shape: inside the window and below the head, so only the
    /// identity can catch it.
    #[test]
    fn a_plausible_cursor_from_another_database_is_rebuilt() {
        let restored = EnvironmentIdentity::new("7667539899605483559:16387");
        assert_ne!(ours(), restored);
        assert_eq!(resume(&ours(), &ours(), 4_000, 3_999), Resume::From(3_999));
        assert_eq!(resume(&ours(), &restored, 4_000, 3_999), Resume::Rebuild);
        assert_eq!(resume(&ours(), &restored, 4_000, 4_000), Resume::Rebuild);
    }

    #[test]
    fn the_digest_changes_with_the_identity_and_not_otherwise() {
        // Including a database differing only in the half a single-source
        // identity would have dropped.
        let same_cluster_other_database = EnvironmentIdentity::new("7667539899605483559:16387");
        let other_cluster_same_oid = EnvironmentIdentity::new("1111111111111111111:16386");
        assert_eq!(ours().digest(), ours().digest());
        assert_ne!(ours().digest(), same_cluster_other_database.digest());
        assert_ne!(ours().digest(), other_cluster_same_oid.digest());
        assert_eq!(ours().digest().len(), 16);
    }
}
