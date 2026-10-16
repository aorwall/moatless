//! The Task listing read model, and the pruner that keeps the listing change
//! feed bounded.
//!
//! The feed itself is [`moatless_core::change_feed::listing`] — it is written
//! from more than one band, so it lives below all of them.

pub mod listed;
mod listed_dao;
mod names_dao;
pub mod operations;
pub mod pruner;

#[cfg(all(test, feature = "db-tests"))]
mod listed_db_tests;
#[cfg(all(test, feature = "db-tests"))]
mod names_db_tests;

pub use listed::{ListedAgent, ListedPullRequest, ListedTask, ListingCursor, RunningChild};
pub use operations::{
    ARCHIVED_LIMIT, BROWSE_LIMIT, FOLLOWED_LIMIT, LISTED_STATUSES, ListingFilter, TaskListing,
    status_list,
};
pub use pruner::spawn_change_pruner;
