//! How many times a bound pull request's host state has changed, which a T3
//! client's `pullRequests.subscribeRefreshes` re-reads on.
//!
//! A count, not a log: a reader learns only that something moved, and
//! re-reads what it shows. Rolled-back writes leave gaps, which cost a reader
//! one spare re-read. A sequence holds no lock, so it sits outside
//! [`super`]'s lock order.

mod dao;

pub use dao::{host_changes, record_host_change};
