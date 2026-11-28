#![cfg(feature = "db-tests")]
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests fail by panicking"
)]
//! What a viewer sees in the T3 compatibility module's shell listing.
//!
//! The listing's two halves are filtered by deliberately different rules, and
//! the difference is invisible without a real database: the access filter, the
//! follow filter and the widening are all SQL. These tests seed workspaces and
//! conversations across ownership, visibility, scope and following, and assert
//! per viewer.

mod change_kinds;
mod contract;
mod conversations;
mod delivery;
mod feed;
mod files;
mod harness;
mod history;
mod projects;
mod pull_requests;
mod reconnect;
mod restore;
mod resume_grouping;
mod row;
mod seeding;
