//! What *happened* in a conversation, as rows.
//!
//! A Message is what somebody said — a role and text. An activity is everything
//! on the way there: a tool call carrying its own outcome, a stretch of thinking,
//! a Sandbox status change, a Sandbox error. An activity is identified by where
//! it came from rather than by when it was written —
//! `{message_uuid}:{block_index}` for a block, the event id for a Sandbox event —
//! which is what makes redelivery a no-op rather than a duplicate.
//!
//! # The counter and the change log
//!
//! Both live here too, because they are how activities and Messages end up in one
//! order and how a reader hears about either. Every write takes a position from
//! that Task's own counter, in the mutation's own transaction, and a write that
//! changed a Message also leaves a record at that position.
//!
//! [`task_listing`](crate::task_listing) answers "which Tasks changed since
//! position N" for a sidebar. The log here answers "what changed *inside* this
//! conversation since position N", which that feed cannot: only Turn transitions
//! enter it, so a Message arriving is invisible there, and its counter is one row
//! for the whole deployment — a per-activity record through it would put every
//! conversation write in the deployment behind one lock.
//!
//! A `bigserial` will not do, for the reason spelled out on that sibling module:
//! an id assigned at INSERT and made visible at COMMIT can be read out of order,
//! so a reader polling past a position that had not committed yet never sees it,
//! silently, while reporting itself up to date.
//!
//! No T3 vocabulary appears here — [`moatless_t3code`] turns a row into the event
//! that surface carries, which is also why a change record names a Message rather
//! than describing what about it changed.

pub mod dao;
pub mod mapping;
pub mod models;
pub mod pruner;
pub mod view;

pub use dao::{
    Activity, changes_from, current_head, prune_changes, record_activity_change,
    record_message_change, record_session_change,
};
pub use models::{
    ConversationChange, ConversationHead, EVENTS_PER_CHANGE, RESUME_WINDOW, RETAINED_CHANGES,
    Resume, position_of, resume,
};
pub use pruner::spawn_change_pruner;
