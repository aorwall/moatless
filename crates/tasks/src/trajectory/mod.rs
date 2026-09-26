//! The trajectory: our own record of every harness Session a Task runs, one
//! immutable event per line of the harness session file.
//!
//! Nothing on the message side reads these tables; the sandbox reaches them
//! only through `AppendTrajectory` and `ReadTrajectory`.

pub(crate) mod dao;
pub(crate) mod lineage;
pub mod models;
pub mod operations;
