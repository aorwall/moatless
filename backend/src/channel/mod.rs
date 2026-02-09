//! Channel module — external messaging integrations (Slack, Email, etc.).
//!
//! Routes inbound messages to tasks and sends outbound replies.

pub mod error;
pub mod handlers;
pub mod inbound;
pub mod outbound;
pub mod provider;
pub mod router;
pub mod routes;
pub mod slack;
pub mod types;
