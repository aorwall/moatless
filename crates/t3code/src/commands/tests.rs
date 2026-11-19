//! Unit tests for the command table, one module per group of commands.

use super::*;
use crate::dao::{InteractiveCall, MockConversationDao};
use async_trait::async_trait;
use moatless_core::change_feed::listing::{EnvironmentIdentity, ListingHead};
use moatless_types::agent::AgentProfile;
use std::sync::Mutex;

mod fixtures;

mod answers;
mod archive_and_revert;
mod attachments;
mod create;
mod discarded_and_refused;
mod fork;
mod issues;
mod lifecycle;
mod meta;
mod models;
mod pull_requests;
mod queue_and_visits;
mod scope;
mod scripts;
mod turns;
