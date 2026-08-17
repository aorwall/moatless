//! Permanent backend facade for Task activities and the conversation feed.
//!
//! Explicit named re-exports (never a glob) so the crate census can resolve
//! edges through the facade. The activity rows come from `moatless-tasks`; the
//! conversation change feed is written from more than one band, so it lives in
//! `moatless-backend-core` below all of them.

pub use moatless_backend_core::change_feed::conversation::{
    ConversationChange, ConversationHead, EVENTS_PER_CHANGE, RESUME_WINDOW, RETAINED_CHANGES,
    Resume, changes_from, current_head, position_of, prune_changes, record_activity_change,
    record_message_change, record_session_change, resume,
};
pub use moatless_tasks::task_activities::{dao, mapping, pruner, spawn_change_pruner, view};
