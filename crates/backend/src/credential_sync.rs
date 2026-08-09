//! Permanent facade re-exporting the `credential_sync` module from
//! `moatless-tasks`.
//!
//! Explicit named re-exports (never a glob) so the crate census can resolve
//! edges through the facade.

pub use moatless_tasks::credential_sync::{refresher, CredentialSync};
