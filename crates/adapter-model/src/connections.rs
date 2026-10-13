//! Adapter connections: which external account an Adapter speaks for, and who
//! may say so.
//!
//! Two authorities run over the same rows and stay apart:
//! [`AdapterConnections`] runs every operation under a viewer's
//! `ScopeContext`; [`ConnectionRouting`] runs unscoped, where an inbound
//! event, not a session, decides the tenant.
//!
//! Both are built by [`crate::compose::adapter_connections`]; providers name a
//! capability trait from [`ports`] instead of linking a database driver.

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
