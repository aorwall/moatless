//! The conversation read model and the Projector that writes it.

pub mod projector;
#[cfg(all(test, feature = "db-tests"))]
mod projector_db_tests;
