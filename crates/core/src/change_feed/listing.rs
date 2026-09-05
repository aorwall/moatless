//! The durable record that a Task's listing-visible state changed.
//!
//! One counter row for the whole deployment, held until commit. See the module
//! doc on [`super`] for why the hold is what makes commit order and position
//! order the same order, and for this counter's place in the lock order.
//!
//! No T3 vocabulary appears here — `moatless-t3code` turns the record into a
//! shell event.

use sha2::{Digest, Sha256};
use sqlx::{PgConnection, PgPool};

/// How far behind a client may be and still be replayed rather than rebuilt.
/// Positions rather than an age, because positions are dense and rows are the
/// cost; 5,000 is ten `CHANGES_PER_POLL` batches.
pub const RESUME_WINDOW: i64 = 5_000;

/// How many of the most recent changes are kept. Wider than [`RESUME_WINDOW`],
/// and the margin is load-bearing: [`resume`] decides off the head and the
/// catch-up poll reads the records a moment later, so equal numbers would let a
/// prune committing between the two delete the record just admitted.
pub const RETAINED_CHANGES: i64 = 2 * RESUME_WINDOW;

/// At compile time rather than in a test, because the failure it prevents is
/// silent. See [`RETAINED_CHANGES`].
const _: () = assert!(RETAINED_CHANGES > RESUME_WINDOW);

/// Which database this deployment's positions were allocated in — the cluster's
/// `system_identifier` and the database's OID, joined. Both halves load-bearing,
/// and neither a row anything writes: an identity stored *in* the database is in
/// the dump, so a restore reports the copy as the original. Decision 7.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentIdentity(String);

impl EnvironmentIdentity {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// A short, opaque name changing exactly when the identity does. The
    /// identity is operational detail, and the descriptor carrying it is public.
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
/// One read, deliberately: read apart, a database replaced between them yields
/// one deployment's position carrying the other's identity — the state
/// [`resume`] refuses, arriving through the check.
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

/// Decide whether a client holding `after` can be replayed. `deployment` is the
/// identity this process read at startup; `database` and `head` are what
/// [`current_head`](super::dao::current_head) reads together, now.
///
/// Three reasons to rebuild, and no two see the same failure (Decision 7): a
/// database other than the one this process started against, whose reissued
/// positions the other two guards find plausible by construction; a position
/// never reached, which no change can carry, so silence would strand the client;
/// and a cursor further behind than [`RESUME_WINDOW`]. Pure in the counter's
/// head, never in the retained rows — see
/// [`current_head`](super::dao::current_head).
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

/// Record that `task_id`'s listing-visible state changed, returning the position
/// allocated for it. `tx` is the connection a transaction is running on — `&mut
/// *tx`, never a pool, which would commit immediately and make the record
/// visible before the mutation it describes.
///
/// # Call this last
///
/// **Take every other lock this transaction needs before calling this, and take
/// none afterwards.** The counter is one row for the whole deployment: taking it
/// *before* a `tasks` row puts this transaction on the opposite side of a lock
/// order from every writer that takes it after, and the pair deadlocks under
/// concurrent load. Last makes the counter the innermost lock of everything that
/// touches it — a total order by construction, needing no agreement between
/// callers — and it shortens the hold on the row the whole deployment serializes
/// on: `the_counter_is_measured_under_concurrent_mutation` prices the same
/// transaction both ways, and last runs about half again as fast.
pub async fn record_change(tx: &mut PgConnection, task_id: &str) -> Result<i64, sqlx::Error> {
    let (position,): (i64,) = sqlx::query_as(
        r#"WITH allocated AS (
               INSERT INTO task_listing_position (id, position) VALUES (TRUE, 1)
               ON CONFLICT (id) DO UPDATE
                 SET position = task_listing_position.position + 1
               RETURNING position
           )
           INSERT INTO task_listing_changes (position, task_id)
           SELECT position, $1 FROM allocated
           RETURNING position"#,
    )
    .bind(task_id)
    .fetch_one(tx)
    .await?;
    Ok(position)
}

/// The highest position this deployment has reached — 0 before the first change
/// — together with the identity of the database it reached it in, from **one**
/// statement, which is why this returns a pair. See [`ListingHead`].
///
/// The counter is the authority here and everywhere below it, never
/// `MAX(task_listing_changes.position)`: [`prune_changes`] keeps the records for
/// a bounded window, so on a deployment older than that window a `MAX` drops
/// back to a position long since passed and hands clients a stale place to
/// resume from, silently. Neither lookup touches a heap, which matters because
/// the subscription poll makes this call every interval.
pub async fn current_head(pool: &PgPool) -> Result<ListingHead, sqlx::Error> {
    // `pg_control_system()` carries no ACL, so an ordinary application role can
    // call it. Verified rather than assumed — `router_from_env` fails startup
    // rather than inventing an identity, so the reverse would be a deployment
    // blocker. Decision 7, and
    // `the_identity_is_readable_by_a_least_privilege_application_role`.
    let (position, identity): (i64, String) = sqlx::query_as(
        r#"SELECT
               COALESCE((SELECT position FROM task_listing_position), 0),
               (SELECT system_identifier::text FROM pg_control_system())
                   || ':'
                   || (SELECT oid::text FROM pg_database
                       WHERE datname = current_database())"#,
    )
    .fetch_one(pool)
    .await?;
    Ok(ListingHead {
        identity: EnvironmentIdentity::new(identity),
        position,
    })
}

/// The counter alone, for callers with no use for the identity beside it.
pub async fn current_position(pool: &PgPool) -> Result<i64, sqlx::Error> {
    Ok(current_head(pool).await?.position)
}

/// Every change recorded after `after`, oldest first, capped at `limit`. Safe as
/// a polling cursor only because commits happen in position order, so nothing
/// can appear below a position this has already returned.
pub async fn changes_after(
    pool: &PgPool,
    after: i64,
    limit: i64,
) -> Result<Vec<ListingChange>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT position, task_id
           FROM task_listing_changes
           WHERE position > $1
           ORDER BY position
           LIMIT $2"#,
    )
    .bind(after)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Delete the changes that have fallen out of the retention window, returning
/// how many went. The cut comes from the counter, per [`current_head`]. Safe
/// from every replica — one bounded `DELETE` over durable state, so two racing
/// delete the same rows and the second finds nothing. No lease, no leader.
pub async fn prune_changes(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let deleted = sqlx::query(
        r#"DELETE FROM task_listing_changes
           WHERE position <= (SELECT position FROM task_listing_position) - $1"#,
    )
    .bind(RETAINED_CHANGES)
    .execute(pool)
    .await?
    .rows_affected();
    Ok(deleted)
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
