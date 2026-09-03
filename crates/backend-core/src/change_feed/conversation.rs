//! The durable record that a Task's conversation moved.
//!
//! One counter row per Task, held until commit. See the module doc on [`super`]
//! for why the hold is load-bearing and for this counter's place in the lock
//! order.
//!
//! [`task_listing`](super::listing) answers "which Tasks changed since position
//! N" for a sidebar. This log answers "what changed *inside* this conversation
//! since position N", which that feed cannot: only Turn transitions enter it, so
//! a Message arriving is invisible there, and its counter is one row for the
//! whole deployment — a per-activity record through it would put every
//! conversation write in the deployment behind one lock.

use sqlx::{PgConnection, PgPool};

/// How far behind a client watching one conversation may be and still be
/// replayed rather than sent the transcript again.
///
/// Positions rather than an age, as in the Task listing feed,
/// and larger than the shell's because a position here is one Message rather
/// than one Task: an agent working for a few minutes spends thousands of them,
/// and a client that reconnects across that has to be replayable or the
/// re-sent transcript is exactly the cost this feed exists to remove.
pub const RESUME_WINDOW: i64 = 20_000;

/// How many of a conversation's most recent changes are kept. Wider than
/// [`RESUME_WINDOW`], and the margin is load-bearing for the reason the shell's
/// is: [`resume`] decides off the head and the catch-up read happens a moment
/// later, so equal numbers would let a prune committing between the two delete
/// the record just promised.
pub const RETAINED_CHANGES: i64 = 2 * RESUME_WINDOW;

/// At compile time rather than in a test, because the failure it prevents is
/// silent. See [`RETAINED_CHANGES`].
const _: () = assert!(RETAINED_CHANGES > RESUME_WINDOW);

/// How many events one change may produce, and the factor separating two
/// consecutive positions on the wire.
///
/// The contract numbers *events*, not changes, and the client drops any event
/// whose sequence it has already passed — so two events sharing a sequence
/// means the second is discarded in silence. One change re-reads one Message
/// and produces its bubble plus one activity per thinking or tool-call block,
/// which is why a position cannot be a sequence on its own.
///
/// Multiplying instead of allocating per event keeps the writer free of any
/// knowledge of what a change projects to — the reader decides that, and can
/// change its mind without a migration. The cost is this ceiling: a Message
/// with more than this many renderable blocks would collide with the next
/// position, so the projection stops there and says so. Anthropic's own limit
/// on tool calls per assistant turn is far below it.
pub const EVENTS_PER_CHANGE: i64 = 1_024;

/// The highest position a conversation has reached — 0 before its first change.
///
/// Unlike the shell's head this carries no database identity. The guard against
/// a restored database reissuing positions is the shell's, read once per
/// snapshot from the same deployment counter; a conversation cursor that
/// survived a restore is refused there before it reaches this comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversationHead {
    pub position: i64,
}

impl ConversationHead {
    /// The sequence a snapshot taken at this head reports, in the numbering
    /// [`EVENTS_PER_CHANGE`] describes. A client resumes from what it last saw,
    /// and a snapshot's sequence and an event's sequence have to be the same
    /// kind of number for that to mean anything.
    ///
    /// This is the head change's *first* event, so a client in step may hold up
    /// to `EVENTS_PER_CHANGE - 1` more than this. Comparing a cursor against it
    /// as an upper bound rebuilds conversations that had nothing to replay.
    pub fn sequence(self) -> i64 {
        self.position * EVENTS_PER_CHANGE
    }
}

/// Where a resuming client is put.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    /// Everything after this sequence is still recorded. Replay from it.
    From(i64),
    /// Further behind than [`RESUME_WINDOW`], or past the last event the head
    /// change can carry. Send the transcript again.
    Rebuild,
}

/// Decide whether a client holding `after` can be replayed against a
/// conversation whose head is `head`.
///
/// Two reasons to rebuild, and they are not the same failure: a sequence never
/// reached, which no change can carry, so silence would strand the client
/// forever; and a cursor further behind than [`RESUME_WINDOW`], whose records
/// have been pruned. A negative cursor is neither — the contract's
/// `NonNegativeInt` refuses it before it is sent, and reading one as absent
/// would rebuild anyway.
///
/// The ceiling is the head change's *last* event, not its first: the head
/// reports index 0 ([`ConversationHead::sequence`]) while a change projects up
/// to [`EVENTS_PER_CHANGE`] events, so a client that saw a turn's tool call
/// holds a sequence above the head and is still exactly in step.
pub fn resume(head: ConversationHead, after: i64) -> Resume {
    let head = head.sequence();
    if after > head + EVENTS_PER_CHANGE - 1 {
        return Resume::Rebuild;
    }
    if head - after > RESUME_WINDOW * EVENTS_PER_CHANGE {
        return Resume::Rebuild;
    }
    Resume::From(after)
}

/// One recorded change: what moved, and where it sits relative to every other
/// change to the same conversation.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ConversationChange {
    pub position: i64,
    /// The Message that changed, or `None` for a change to the conversation
    /// itself — its Turn or its Sandbox, which together are its session.
    pub message_uuid: Option<String>,
    /// The activity that changed, when it happened on no Message — a Sandbox
    /// lifecycle change, a context-window reading. At most one of this and
    /// `message_uuid` is set; neither means the conversation's own session moved.
    pub activity_id: Option<String>,
}

impl ConversationChange {
    /// The sequence the `index`th event projected from this change carries.
    /// `None` past [`EVENTS_PER_CHANGE`], where the next position begins.
    pub fn sequence(&self, index: usize) -> Option<i64> {
        let index = i64::try_from(index).ok()?;
        (index < EVENTS_PER_CHANGE).then_some(self.position * EVENTS_PER_CHANGE + index)
    }
}

/// The position a cursor sits in, for the read that catches a client up.
///
/// Floor division rather than the next position: a client disconnected halfway
/// through one change's events holds a sequence inside that change, and reading
/// from the position *after* it would drop the rest. The read starts at the
/// position the cursor is inside and the events themselves are filtered by
/// sequence, so a whole-position cursor re-reads a change it has fully seen and
/// emits nothing from it.
pub fn position_of(sequence: i64) -> i64 {
    sequence.div_euclid(EVENTS_PER_CHANGE)
}

// ── The conversation's position counter and change log ──────────────────────
//
// Activities and Messages are ordered by one counter per Task, and a reader
// learns the conversation moved from the log that counter writes.

/// Record that `message_uuid` was written or rewritten, returning the position
/// allocated for it. `tx` is the connection the mutation's transaction is
/// running on — `&mut *tx`, never a pool, which would commit immediately and
/// make the record visible before the write it describes.
///
/// # Call this before `task_listing::record_change`
///
/// Both allocators hold a counter row until commit, so every transaction that
/// takes both must take them in one order or the pair deadlocks. This one goes
/// first and the shell's stays last, exactly as its own "call this last" says:
/// this counter is one row per Task and the shell's is one row for the whole
/// deployment, so this order also holds the contended one for the shorter time.
pub async fn record_message_change(
    tx: &mut PgConnection,
    task_id: &str,
    message_uuid: &str,
) -> Result<i64, sqlx::Error> {
    allocate(tx, task_id, Some(message_uuid), None).await
}

/// Record that the conversation itself moved — its Turn, or the Sandbox its
/// session status is read from. Names no Message, because none changed: the
/// reader answers this by re-reading the conversation's own row.
///
/// Same transaction rule and same ordering against the shell's allocator as
/// [`record_message_change`].
pub async fn record_session_change(
    tx: &mut PgConnection,
    task_id: &str,
) -> Result<i64, sqlx::Error> {
    allocate(tx, task_id, None, None).await
}

/// Record that an activity happened on no Message — a Sandbox lifecycle change,
/// a context-window reading. Names the activity so a live reader re-reads that
/// one row rather than the conversation's own session, which nothing about the
/// activity moved.
///
/// Same transaction rule and same ordering against the shell's allocator as
/// [`record_message_change`].
pub async fn record_activity_change(
    tx: &mut PgConnection,
    task_id: &str,
    activity_id: &str,
) -> Result<i64, sqlx::Error> {
    allocate(tx, task_id, None, Some(activity_id)).await
}

/// Take the next position without recording a change.
///
/// Activities are ordered by this counter and notified through the Message they
/// belong to, so each one takes a position but leaves no record: a record per
/// activity would multiply the feed's row count by the number of blocks in a
/// turn to say something the Message's own record already said.
///
/// One position per activity rather than one shared by all of a Message's is
/// what keeps `(task_id, position)` a total order. Sharing one would leave
/// activities within a Message tied, and the only tiebreaker left is the id —
/// where `msg:10` sorts before `msg:2`.
///
/// Same transaction rule as [`record_message_change`], and no ordering hazard
/// against it: the first allocation in a transaction takes the counter row and
/// holds it until commit, so every later one on the same connection is free.
pub async fn allocate_position(tx: &mut PgConnection, task_id: &str) -> Result<i64, sqlx::Error> {
    let (position,): (i64,) = sqlx::query_as(
        r#"INSERT INTO task_conversation_position (task_id, position) VALUES ($1, 1)
           ON CONFLICT (task_id) DO UPDATE
             SET position = task_conversation_position.position + 1
           RETURNING position"#,
    )
    .bind(task_id)
    .fetch_one(tx)
    .await?;
    Ok(position)
}

/// The one allocating statement, shared so the two subjects cannot drift into
/// two different orderings of the same counter row. The `ON CONFLICT DO UPDATE`
/// is the allocation, and it holds that row for the rest of the transaction.
async fn allocate(
    tx: &mut PgConnection,
    task_id: &str,
    message_uuid: Option<&str>,
    activity_id: Option<&str>,
) -> Result<i64, sqlx::Error> {
    let (position,): (i64,) = sqlx::query_as(
        r#"WITH allocated AS (
               INSERT INTO task_conversation_position (task_id, position) VALUES ($1, 1)
               ON CONFLICT (task_id) DO UPDATE
                 SET position = task_conversation_position.position + 1
               RETURNING position
           )
           INSERT INTO task_conversation_changes (task_id, position, message_uuid, activity_id)
           SELECT $1, position, $2, $3 FROM allocated
           RETURNING position"#,
    )
    .bind(task_id)
    .bind(message_uuid)
    .bind(activity_id)
    .fetch_one(tx)
    .await?;
    Ok(position)
}

/// The highest position this conversation has reached — 0 before its first
/// change, and 0 for a Task that has never had one.
///
/// The counter is the authority, never `MAX(task_conversation_changes.position)`:
/// [`prune_changes`] keeps the records for a bounded window, so on a
/// conversation older than that window a `MAX` drops back to a position long
/// since passed and hands a client a stale place to resume from, silently.
pub async fn current_head(pool: &PgPool, task_id: &str) -> Result<ConversationHead, sqlx::Error> {
    let position: Option<(i64,)> =
        sqlx::query_as(r#"SELECT position FROM task_conversation_position WHERE task_id = $1"#)
            .bind(task_id)
            .fetch_optional(pool)
            .await?;
    Ok(ConversationHead {
        position: position.map_or(0, |(position,)| position),
    })
}

/// Every change to this conversation recorded at or after `from`, oldest first,
/// capped at `limit`.
///
/// At `from` rather than after it: a client disconnected partway through one
/// change's events holds a cursor inside that position, and the events it
/// missed are the rest of that same change. Filtering the *events* by sequence
/// is what makes this exact — see
/// [`position_of`](super::models::position_of).
///
/// Safe as a polling cursor only because commits happen in position order, so
/// nothing can appear below a position this has already returned.
pub async fn changes_from(
    pool: &PgPool,
    task_id: &str,
    from: i64,
    limit: i64,
) -> Result<Vec<ConversationChange>, sqlx::Error> {
    sqlx::query_as(
        r#"SELECT position, message_uuid, activity_id
           FROM task_conversation_changes
           WHERE task_id = $1 AND position >= $2
           ORDER BY position
           LIMIT $3"#,
    )
    .bind(task_id)
    .bind(from)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Delete the changes that have fallen out of each conversation's retention
/// window, returning how many went. The cut comes from each Task's own counter,
/// per [`current_head`]. Safe from every replica — one bounded `DELETE` over
/// durable state, so two racing delete the same rows and the second finds
/// nothing. No lease, no leader.
pub async fn prune_changes(pool: &PgPool) -> Result<u64, sqlx::Error> {
    let deleted = sqlx::query(
        r#"DELETE FROM task_conversation_changes c
           USING task_conversation_position p
           WHERE c.task_id = p.task_id
             AND c.position <= p.position - $1"#,
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

    fn head(position: i64) -> ConversationHead {
        ConversationHead { position }
    }

    #[test]
    fn a_cursor_is_replayable_up_to_the_resume_window_and_no_further() {
        assert_eq!(resume(head(0), 0), Resume::From(0));
        assert_eq!(resume(head(10), head(10).sequence()), Resume::From(10_240));
        assert_eq!(resume(head(10), 0), Resume::From(0));
        // Exactly the window, and one position past it.
        assert_eq!(resume(head(RESUME_WINDOW), 0), Resume::From(0));
        assert_eq!(resume(head(RESUME_WINDOW + 1), 0), Resume::Rebuild);
    }

    /// A cursor from another conversation, or from a database whose positions
    /// were reissued: plausible in shape, ahead of anything allocated here.
    /// Past the head change's events, not merely past its first one.
    #[test]
    fn a_cursor_ahead_of_the_head_is_rebuilt() {
        assert_eq!(resume(head(0), EVENTS_PER_CHANGE), Resume::Rebuild);
        assert_eq!(
            resume(head(10), head(10).sequence() + EVENTS_PER_CHANGE),
            Resume::Rebuild
        );
    }

    /// The head reports the head change's first event, but that change projects
    /// a bubble plus one event per thinking or tool-call block — so a client in
    /// step holds a sequence above it. Reading that as "ahead of anything
    /// allocated" re-sent the whole transcript on every resubscribe.
    #[test]
    fn a_cursor_inside_the_head_change_replays_rather_than_rebuilding() {
        let head = head(10);
        for index in 0..EVENTS_PER_CHANGE {
            let after = head.sequence() + index;
            assert_eq!(
                resume(head, after),
                Resume::From(after),
                "a client holding event {index} of the head change was rebuilt"
            );
            // The catch-up read stays inside the head change, so it replays
            // that change and emits only what sorts past the cursor.
            assert_eq!(position_of(after), head.position);
        }
    }

    /// The client drops an event whose sequence it has already passed, so two
    /// events may never share one — including across a position boundary.
    #[test]
    fn consecutive_positions_never_reuse_a_sequence() {
        let first = ConversationChange {
            position: 4,
            message_uuid: None,
            activity_id: None,
        };
        let second = ConversationChange {
            position: 5,
            message_uuid: None,
            activity_id: None,
        };
        assert_eq!(first.sequence(0), Some(4 * EVENTS_PER_CHANGE));
        assert!(first.sequence(0) < first.sequence(1));
        let last = first.sequence(usize::try_from(EVENTS_PER_CHANGE - 1).unwrap());
        assert!(last < second.sequence(0));
        // One past the ceiling has nowhere to go rather than colliding.
        assert_eq!(
            first.sequence(usize::try_from(EVENTS_PER_CHANGE).unwrap()),
            None
        );
    }

    /// A cursor inside a change reads that change again rather than skipping
    /// the events it had not received.
    #[test]
    fn a_cursor_inside_a_change_reads_that_change_again() {
        let change = ConversationChange {
            position: 7,
            message_uuid: None,
            activity_id: None,
        };
        let halfway = change.sequence(3).unwrap();
        assert_eq!(position_of(halfway), 7);
        // And a cursor exactly on a boundary sits in the change it completed.
        assert_eq!(position_of(change.sequence(0).unwrap()), 7);
        assert_eq!(position_of(0), 0);
    }
}
