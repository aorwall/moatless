//! Persistent pod lifecycle watcher.
//!
//! Watches sandbox pods (`app.kubernetes.io/managed-by=moatless`) for crashes,
//! evictions, and restarts, publishing to Redis Streams; only the leader (K8s
//! Lease) runs it.
//!
//! Pod states key by `pod_name`, not `task_id`, so a rolling update tracks the
//! terminating and starting pod independently; a delete checks for a surviving
//! sibling before publishing `pod_deleted` or `NotCreated`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use futures::TryStreamExt;
use k8s_openapi::api::core::v1::{Event as CoreEvent, Pod};
use kube::api::{Api, DeleteParams, ListParams, LogParams};
use kube::runtime::watcher::{self, Config as WatcherConfig};
use moatless_types::sandbox::{
    SandboxEventPayload, SandboxStatus, ServerEventPayload, ServerSubType,
};
use moatless_types::task::TaskTurnSubType;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::client::K8sClient;
use super::deployment;
use super::leader_election::LeaderElection;
use super::pod_observation_policy::{
    ObservationContext, PodObservationPolicy, cause_is_attach_failure,
    should_attempt_storage_fallback, should_emit_scheduling_blocked, should_emit_startup_slow,
    should_gc_failed_pod,
};
use super::pod_status::{extract_container_observed_statuses, is_sandbox_container_ready};
use super::provider::{StorageFallback, StorageFallbackOutcome};
use moatless_core::events::EventPublisher;
use moatless_sandbox_control::lifecycle::Lifecycle;
use moatless_sandbox_control::lifecycle::watcher::classify_observation;
use moatless_sandbox_runtime::classify::ObservedTransition;
use moatless_sandbox_runtime::container_status::DEFAULT_SERVER_READINESS_TIMEOUT_SECS;
use moatless_sandbox_runtime::observation_events::publish_sandbox_observation;
use moatless_sandbox_runtime::operation::OperationGeneration;
use moatless_sandbox_runtime::runtime::{OPERATION_GENERATION_LABEL, generation_from_label};
use moatless_types::sandbox::{SandboxDesiredState, SandboxStopReason};

mod diagnostics;
mod event;
mod observed;
mod remediation;
mod sweep;
mod watch;

#[cfg(test)]
mod tests;

pub use watch::spawn_pod_lifecycle_watcher;

pub(crate) use observed::{PodObservedState, ServerContainerState, extract_pod_observed_state};
pub(crate) use sweep::{
    FAILED_POD_GC_GRACE_MS, SCHEDULING_BLOCKED_THRESHOLD_SECS, STARTUP_SLOW_THRESHOLD_SECS,
};
