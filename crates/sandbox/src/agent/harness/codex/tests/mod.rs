//! Unit + gated E2E coverage for the Codex adapter: native item normalization
//! (one source event per item family, collision-safe ids, usage retention),
//! failure classification, and the one-shot lifecycle through the shared driver
//! (fresh Turn ordering, queued input, cancellation teardown, unexpected exit).
//!
//! The item-family tests are driven from the checked-in transport fixtures
//! (`codex_app_server/fixtures/`), not from hand-written JSON: the mapping is
//! only worth anything if it is built against what the server actually sends.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::agent::codex_app_server::protocol::TurnError;
use crate::agent::codex_app_server::{AppServerSession, ServerEvent};
use crate::agent::types::ResumeSession;
use crate::credentials::recovery::{CODEX_AUTH_JSON, CredentialFailureDetector};

use super::super::classify::{
    classify_startup_error, classify_stream_error, classify_turn_error, is_expired_credential,
};
use super::{CodexConfig, CodexProcess, MID_TURN_DELIVERY_TIMEOUT};
use crate::agent::harness::Harness;
use crate::agent::harness::error::{ErrorClass, FailureStage, SessionImpact};
use crate::agent::harness::process::{AgentProcess, ProcessEvent, ProcessTermination};
use crate::agent::harness::signal::{Cause, Input, InputId, Outcome, Signal};
use crate::agent::harness::source::{Role, SourceEvent};

mod fixtures;
mod normalization;
mod threads;
mod usage;
mod classification;
mod steering;
mod driver;
mod e2e;
