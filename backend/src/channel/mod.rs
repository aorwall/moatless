//! Channel module — external messaging integrations (Slack, Webhook, Cron, etc.).
//!
//! Routes inbound messages to tasks and sends outbound replies.

pub mod config_dao;
pub mod cron;
pub mod defaults;
pub mod error;
pub mod external_id;
pub mod github_pr;
pub mod handlers;
pub mod inbound;
pub mod ingress_rule_dao;
pub mod listen;
pub mod models;
pub mod outbound;
pub mod provider;
pub mod provider_connection_dao;
pub mod recently_sent;
pub mod router;
pub mod routes;
pub mod schemas;
pub mod slack;
pub mod task_subscription_dao;
pub mod telegram;
pub mod thread_dao;
pub mod webhook;
