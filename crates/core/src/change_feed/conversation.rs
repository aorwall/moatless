//! Conversation change-feed positions and persistence.

mod dao;
mod position;
pub use dao::{record_message_change, record_session_change, record_activity_change, record_revert_change, allocate_position, current_head, changes_from, prune_changes};
pub use position::{RESUME_WINDOW, RETAINED_CHANGES, EVENTS_PER_CHANGE, ConversationHead, Resume, resume, ConversationChange, position_of};
