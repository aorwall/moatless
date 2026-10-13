//! The conversation read model and the Projector that writes it.

pub(crate) mod assemble;
pub(crate) mod dao;
pub(crate) mod ids;
pub(crate) mod payload;
pub mod projector;
#[cfg(all(test, feature = "db-tests"))]
mod projector_db_tests;
pub(crate) mod read_dao;
mod replay;
pub(crate) mod replay_dao;
#[cfg(all(test, feature = "db-tests"))]
mod replay_db_tests;
pub mod transcript;
pub(crate) mod transcript_dao;
