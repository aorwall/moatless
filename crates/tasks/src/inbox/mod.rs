//! The Task inbox: every message addressed to the agent, and whether it has
//! been delivered. Pending work is read here, never from the conversation.

pub(crate) mod dao;
#[cfg(all(test, feature = "db-tests"))]
mod dao_db_tests;
pub mod models;
mod service;
#[cfg(all(test, feature = "db-tests"))]
mod trigger_db_tests;
pub mod view;

pub use models::{
    AdapterChannel, DeliveryOptions, ExternalSender, InboxContext, InboxItem, InboxOrigin,
    InboxStatus, NewInboxItem, RunStamp,
};
pub(crate) use service::{InboxService, PENDING_LIMIT};
