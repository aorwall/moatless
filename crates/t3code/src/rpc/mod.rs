//! Decode, dispatch and supervise UI-RPC sessions and streams.

pub(crate) mod config;
mod dispatch;
pub(crate) mod envelope;
pub(crate) mod errors;
pub(crate) mod metrics;
pub(crate) mod session;
pub(crate) mod socket;
mod watch;
