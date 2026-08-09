//! Permanent facade re-exporting the `loops` module from `moatless-loops`.
//!
//! The backend stays the composition root; the provider and server bands still
//! in the backend keep naming `crate::loops::…`. Explicit named re-exports
//! (never a glob) so the crate census resolves edges through the facade.

pub use moatless_loops::loops::{dao, firing, http, sync};
