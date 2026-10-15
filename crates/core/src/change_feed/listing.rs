//! Listing change-feed positions and persistence.

mod dao;
mod position;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub use dao::{changes_after, current_head, current_position, prune_changes, record_change};
pub use position::{
    EnvironmentIdentity, ListingChange, ListingHead, RESUME_WINDOW, RETAINED_CHANGES, Resume,
    resume,
};
