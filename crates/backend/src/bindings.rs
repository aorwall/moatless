//! Permanent facade re-exporting the `bindings` module from `moatless-loops`.
//!
//! The backend stays the composition root; the provider and server bands still
//! in the backend keep naming `crate::bindings::…`. Explicit named re-exports
//! (never a glob) so the crate census resolves edges through the facade.

pub use moatless_loops::bindings::{
    dao, footer, handlers, metrics, outbound, recently_sent, recipients, reply, runtime, schemas,
    status_service,
};
