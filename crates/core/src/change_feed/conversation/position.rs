//! The durable record that a Task's conversation moved.
//!
//! One counter row per Task, held until commit — see [`super::super`] for the
//! lock order.
//!
//! [`task_listing`](super::super::listing) answers "which Tasks changed since
//! position N". This answers "what changed *inside* this conversation since
//! position N": only Turn transitions enter the listing feed, so a Message
//! arriving there is invisible.

/// How far behind a client watching one conversation may be and still be
/// replayed rather than sent the transcript again.
///
/// Counts positions, not age: a position here is one Message, and an agent
/// working a few minutes produces thousands.
pub const RESUME_WINDOW: i64 = 20_000;

/// How many of a conversation's most recent changes are kept.
///
/// Must stay wider than [`RESUME_WINDOW`], or a prune could delete the record
/// [`resume`] just promised to replay.
pub const RETAINED_CHANGES: i64 = 2 * RESUME_WINDOW;

/// At compile time rather than in a test, because the failure it prevents is
/// silent. See [`RETAINED_CHANGES`].
const _: () = assert!(RETAINED_CHANGES > RESUME_WINDOW);

/// How many events one change may produce, and the factor between two
/// consecutive positions on the wire.
///
/// Two events must never share a sequence; a Message with more renderable
/// blocks than this has its projection stop here instead of colliding with the
/// next position.
pub const EVENTS_PER_CHANGE: i64 = 1_024;

/// The highest position a conversation has reached — 0 before its first change.
///
/// Carries no database identity; the restore guard on the shell's head rejects
/// a surviving cursor before it reaches this comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConversationHead {
    pub position: i64,
}

impl ConversationHead {
    /// The sequence a snapshot at this head reports, in [`EVENTS_PER_CHANGE`]'s
    /// numbering.
    ///
    /// This is the head change's *first* event; a client in step may hold up to
    /// `EVENTS_PER_CHANGE - 1` more, so comparing it as an upper bound wrongly
    /// rebuilds conversations with nothing left to replay.
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

/// Decide whether a client holding `after` can be replayed against `head`.
///
/// Rebuilds if `after` is past the head change's *last* event (unlike
/// [`ConversationHead::sequence`]'s first) or further behind than
/// [`RESUME_WINDOW`] and already pruned. Position 0 is the exception.
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
    /// `message_uuid` is set; neither means the conversation's own session
    /// moved.
    pub activity_id: Option<String>,
    /// The turn the conversation was rewound to, for the one change that names
    /// no surviving row: everything above it was deleted, 0 meaning none
    /// survived.
    ///
    /// Set only alongside two `None`s — a reader re-reads what a change names,
    /// and this one names what is gone.
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
/// Floor division, not the next position: a disconnected client holds a
/// sequence mid-change, and starting after that position would drop the rest.
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
    /// there is no head change whose events a client could hold. Only the
    /// cursor that has seen nothing replays; any positive cursor must rebuild,
    /// or it opens a feed on an empty change log that sends nothing.
    #[test]
    fn an_empty_head_replays_only_the_seen_nothing_cursor() {
        assert_eq!(resume(head(0), 0), Resume::From(0));
        assert_eq!(resume(head(0), 1), Resume::Rebuild);
        assert_eq!(resume(head(0), EVENTS_PER_CHANGE - 1), Resume::Rebuild);
    }

    /// The head reports the head change's first event, but that change projects
    /// a bubble plus one event per thinking or tool-call block, so a client in
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
