//! Permanent facade re-exporting the `task_activities` module from
//! `moatless-tasks`.
//!
//! Explicit named re-exports (never a glob) so the crate census can resolve
//! edges through the facade. Mirrors both the submodules and the crate's own
//! top-level re-exports from `dao`, `models`, and `pruner`.

pub use moatless_tasks::task_activities::{
    changes_from, current_head, dao, mapping, models, position_of, prune_changes, pruner,
    record_activity_change, record_message_change, record_session_change, resume,
    spawn_change_pruner, view, ConversationChange, ConversationHead, Resume, EVENTS_PER_CHANGE,
    RESUME_WINDOW, RETAINED_CHANGES,
};
