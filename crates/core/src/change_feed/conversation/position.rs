//! The durable record that a Task's conversation moved.
//!
//! One counter row per Task, held until commit. See the module doc on [`super::super`]
//! for why the hold is load-bearing and for this counter's place in the lock
//! order.
//!
//! [`task_listing`](super::super::listing) answers "which Tasks changed since position
//! N" for a sidebar. This log answers "what changed *inside* this conversation
//! since position N", which that feed cannot: only Turn transitions enter it, so
//! a Message arriving is invisible there, and its counter is one row for the
//! whole deployment — a per-activity record through it would put every
//! conversation write in the deployment behind one lock.

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
///
/// Position 0 is the exception: it means no change was ever allocated
/// (`allocate_position` starts at 1), so there is no head change to hold events,
/// and only the cursor that has seen nothing (0) is in step. Widening the
/// ceiling there would admit a stray cursor from another conversation and open a
/// feed on an empty change log that replays nothing — the strand this rebuilds
/// to avoid.
pub fn resume(head: ConversationHead, after: i64) -> Resume {
    let head = head.sequence();
    let last_replayable = if head == 0 {
        head
    } else {
        head + EVENTS_PER_CHANGE - 1
    };
    if after > last_replayable {
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
    /// The turn the conversation was rewound to, for the one change that names
    /// no surviving row: everything above it was deleted. 0 means none
    /// survived. Set only alongside two `None`s — a reader re-reads what a
    /// change names, and this one names what is gone.
    pub reverted_to_turn: Option<i32>,
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

    /// Position 0 is the empty-head sentinel: no change was ever allocated, so
    /// there is no head change whose events a client could hold. Only the cursor
    /// that has seen nothing replays; any positive cursor is from elsewhere and
    /// must rebuild, or it opens a feed on an empty change log that sends nothing.
    #[test]
    fn an_empty_head_replays_only_the_seen_nothing_cursor() {
        assert_eq!(resume(head(0), 0), Resume::From(0));
        assert_eq!(resume(head(0), 1), Resume::Rebuild);
        assert_eq!(resume(head(0), EVENTS_PER_CHANGE - 1), Resume::Rebuild);
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
            reverted_to_turn: None,
        };
        let second = ConversationChange {
            position: 5,
            message_uuid: None,
            activity_id: None,
            reverted_to_turn: None,
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
            reverted_to_turn: None,
        };
        let halfway = change.sequence(3).unwrap();
        assert_eq!(position_of(halfway), 7);
        // And a cursor exactly on a boundary sits in the change it completed.
        assert_eq!(position_of(change.sequence(0).unwrap()), 7);
        assert_eq!(position_of(0), 0);
    }
}
