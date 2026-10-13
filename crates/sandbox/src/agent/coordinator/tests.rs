//! Coordinator seam-mapping contract tests.
//!
//! Each test drives the real [`Coordinator`] over a scripted [`HarnessRuntime`] and a
//! real [`GrpcEventPublisher`] whose outbox delivers to an in-memory recording
//! backend. Assertions are on the exact `agent.*` events published (their
//! order, sub-types, and per-block payload shape) plus the acknowledgement —
//! the externally observable contract, not the coordinator's internals.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use sandbox_proto::{DownloadUploadResponse, GetCredentialsResponse, PublishEventRequest};
use tokio::sync::{RwLock, mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use tonic::Status;

use crate::agent::agent_state::AgentState;
use crate::agent::handle::AgentCommand;
use crate::agent::harness::content::CanonicalContentBlock;
use crate::agent::harness::error::FailureStage;
use crate::agent::harness::{
    Activity, Cause, ErrorClass, HarnessError, HarnessRuntime, Input, InputId, Outcome, Script,
    SessionId, SessionImpact, Signal, Step,
};
use crate::agent::heartbeat::Heartbeat;
use crate::agent::scheduled_job::{JobInjector, LateInjector, ScheduledJobRegistry};
use crate::agent::types::{ResumeSession, StopSource};
use crate::backend_client::BackendClient;
use crate::config::{Config, test_support::minimal_config};
use crate::credentials::recovery::{CODEX_AUTH_JSON, CredentialFailureDetector};
use crate::credentials::retry::MAX_CREDENTIAL_RECOVERY_RETRIES;
use crate::credentials::{CredentialSink, Dispatcher};
use crate::events::GrpcEventPublisher;
use crate::outbox::EventOutbox;

use super::seam::context_recovery_prompt;
use super::service::build_resume;
use super::{Coordinator, CoordinatorService};

mod acks;
mod context_recovery;
mod credentials;
mod fixtures;
mod profile_switch;
mod resume_target;
mod service_fixtures;
mod service_retry;
mod service_stop;
mod service_turns;
mod service_wakeup;
mod turn_events;
