//! Channel module — external messaging integrations (Slack, Webhook, Cron, etc.).
//!
//! Routes inbound messages to tasks and sends outbound replies.

pub mod config_dao;
pub mod cron;
pub mod defaults;
pub mod error;
pub mod handlers;
pub mod inbound;
pub mod listen;
pub mod models;
pub mod outbound;
pub mod provider;
pub mod router;
pub mod routes;
pub mod schemas;
pub mod github_pr;
pub mod slack;
pub mod telegram;
pub mod thread_dao;
pub mod webhook;
