//! The trajectory: our own record of every harness Session a Task runs, one
//! immutable event per line of the harness session file.
//!
//! The Projector reads an appended line's `content` in the append's own
//! transaction; the sandbox reaches these tables only through
//! `AppendTrajectory` and `ReadTrajectory`.

pub(crate) mod dao;
pub(crate) mod lineage;
pub mod models;
pub mod operations;
