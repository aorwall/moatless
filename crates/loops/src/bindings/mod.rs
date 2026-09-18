//! Task bindings and outbound replies to external subjects.

pub mod buffer;
pub(crate) mod dao;
pub mod footer;
pub mod handlers;
pub mod metrics;
pub mod operations;
pub mod outbound;
pub mod ports;
pub mod recently_sent;
pub mod recipients;
pub mod reply;
pub mod runtime;
pub mod schemas;
pub mod status_service;

pub(crate) mod persistence;

pub mod access;
pub mod connect;
pub(crate) mod projection;

pub mod creation;
mod reactions;

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
