//! What a conversation change names, and the rule for whether a client may
//! resume from where it left off.

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
    pub fn sequence(self) -> i64 {
        self.position * EVENTS_PER_CHANGE
    }
}

/// Where a resuming client is put.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resume {
    /// Everything after this sequence is still recorded. Replay from it.
    From(i64),
    /// Further behind than [`RESUME_WINDOW`], or ahead of a sequence this
    /// conversation has ever reached. Send the transcript again.
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
pub fn resume(head: ConversationHead, after: i64) -> Resume {
    let head = head.sequence();
    if after > head {
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
    #[test]
    fn a_cursor_ahead_of_the_head_is_rebuilt() {
        assert_eq!(resume(head(0), 1), Resume::Rebuild);
        assert_eq!(resume(head(10), head(10).sequence() + 1), Resume::Rebuild);
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
