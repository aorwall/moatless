//! Adapter connections: which external account an Adapter speaks for, and who
//! may say so.
//!
//! Two authorities run over the same rows and stay apart:
//! [`AdapterConnections`] runs every operation under a viewer's `ScopeContext`
//! (behind `/api/v1/adapters/connections`); [`ConnectionRouting`] runs
//! unscoped, for paths where an inbound event, not a session, decides the
//! tenant.
//!
//! Both are built by [`crate::compose::adapter_connections`]. A provider crate
//! compiles no database driver, so it names a capability trait from [`ports`]
//! instead.

#[cfg(feature = "postgres")]
pub(crate) mod dao;
#[cfg(feature = "postgres")]
mod operations;
mod ports;

#[cfg(feature = "postgres")]
pub use operations::{AdapterConnections, ConnectionGraph, ConnectionRouting};
pub use ports::{
    ConnectionInstaller, ConnectionLookup, ConnectionReader, connection_for_auth,
    resolve_chat_provider_event, resolve_connection,
};

#[cfg(test)]
mod tests;
