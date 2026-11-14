//! Conversation change-feed positions and persistence.

mod dao;
mod position;
pub use dao::{
    allocate_position, changes_from, current_head, current_heads, prune_changes,
    record_activity_change, record_message_change, record_pull_request_change,
    record_revert_change, record_session_change,
};
pub use position::{
    ConversationChange, ConversationHead, EVENTS_PER_CHANGE, RESUME_WINDOW, RETAINED_CHANGES,
    Resume, position_of, resume,
};
