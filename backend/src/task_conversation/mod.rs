//! The durable record that one Task's conversation changed.
//!
//! [`task_listing`](crate::task_listing) answers "which Tasks changed since
//! position N" for a sidebar. This answers "what changed *inside* this
//! conversation since position N", which that feed cannot: only Turn
//! transitions enter it, so a Message arriving is invisible there, and its
//! counter is one row for the whole deployment — a per-block record through it
//! would put every message write in the deployment behind one lock.
//!
//! The record is written by the code performing the mutation, in that
//! mutation's own transaction, and its position is allocated there too, from
//! that Task's own counter row held until commit. A `bigserial` will not do,
//! for the reason spelled out on the sibling module: an id assigned at INSERT
//! and made visible at COMMIT can be read out of order, so a reader polling
//! past a position that had not committed yet never sees it, silently, while
//! reporting itself up to date.
//!
//! No T3 vocabulary appears here — [`crate::ui_rpc`] turns the record into a
//! conversation event, which is also why the record names a Message rather than
//! describing what about it changed.

pub mod dao;
pub mod models;
pub mod pruner;

pub use dao::{changes_from, current_head, prune_changes, record_message_change,
    record_session_change};
pub use models::{
    position_of, resume, ConversationChange, ConversationHead, Resume, EVENTS_PER_CHANGE,
    RESUME_WINDOW, RETAINED_CHANGES,
};
pub use pruner::spawn_change_pruner;
