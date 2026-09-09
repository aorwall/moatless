//! Reading a stopped task's workspace out of the `restic` repository S3 holds.
//!
//! `tree` is the path handling the three reads share, so a listing is
//! byte-identical whichever one answered.

mod restic;
mod tree;

pub use restic::ResticWorkspaceSnapshots;
