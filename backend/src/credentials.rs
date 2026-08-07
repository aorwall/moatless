//! Credential storage and the codex OAuth refresh flow — the implementation lives in
//! the `moatless-workspaces` crate.
//!
//! Permanent facade (see `repository.rs` for the rationale and the glob caveat).
//! Re-exports the `credentials` module root: its two submodules and the top-level
//! types the backend composes.

pub use moatless_workspaces::credentials::{
    CredentialError, CredentialService, StoreOutcome, WriteSource, codex, error, should_broadcast,
};
