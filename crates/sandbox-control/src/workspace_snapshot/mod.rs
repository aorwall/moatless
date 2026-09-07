//! Reading a stopped task's workspace out of what S3 holds.
//!
//! Two formats back the same three reads. `tree` is the path handling they
//! share, so a listing is byte-identical whichever one answered, and `chain`
//! picks the first backing that holds the task.

mod chain;
mod mirror;
mod restic;
mod tree;

pub use chain::WorkspaceSnapshotChain;
pub use mirror::S3WorkspaceSnapshots;
pub use restic::ResticWorkspaceSnapshots;
