//! Text search over the open Tasks a viewer may read: their Messages and
//! their names.

mod dao;
pub mod matched;
pub mod operations;

#[cfg(all(test, feature = "db-tests"))]
mod db_tests;

pub use matched::TaskMatch;
pub use operations::TaskSearch;
