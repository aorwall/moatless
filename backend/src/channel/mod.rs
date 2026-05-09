//! Channel module — external messaging integrations (Slack, Webhook, Cron, etc.).
//!
//! Routes inbound messages to tasks and sends outbound replies.

pub mod binding_runtime;
pub mod cron;
pub mod dao;
pub mod error;
pub mod github_pr;
pub mod handlers;
pub mod inbound;
pub mod linear;
pub mod listen;
pub mod models;
pub mod outbound;
pub mod provider;
pub mod recently_sent;
pub mod routes;
pub mod schemas;
pub mod services;
pub mod slack;
pub mod telegram;
pub mod webhook;
