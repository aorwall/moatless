//! Listing change-feed positions and persistence.

mod dao;
mod position;
pub use dao::{record_change, current_head, current_position, changes_after, prune_changes};
pub use position::{RESUME_WINDOW, RETAINED_CHANGES, EnvironmentIdentity, ListingHead, Resume, resume, ListingChange};
