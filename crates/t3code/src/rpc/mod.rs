//! Decode, dispatch and supervise UI-RPC sessions and streams.

pub(crate) mod config;
pub(crate) mod dispatch;
pub(crate) mod envelope;
pub(crate) mod errors;
pub(crate) mod metrics;
pub(crate) mod session;
pub(crate) mod socket;
pub(crate) mod table;
pub(crate) mod watch;
