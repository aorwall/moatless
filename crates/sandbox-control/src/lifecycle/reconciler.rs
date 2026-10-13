//! The desired-state reconciler — the single async recovery owner for level
//! states.
//!
//! Each tick compares intent against fleet status via
//! [`reconcile::decide`]; a divergence is confirmed with an authoritative
//! per-Task read before it drives a lifecycle operation, retried via
//! [`AttemptBudget`] until permanent failure or Closed.
//!
//! Holds no lease logic itself: [`Reconciler::reconcile_once`] is
//! lease-elected on Kubernetes, lease-free single-replica on Docker.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tracing::{debug, info, warn};

use moatless_types::sandbox::SandboxStopReason;

use moatless_sandbox_runtime::fleet::{FleetObserver, RuntimeObservation};

use super::attempt::AttemptBudget;
use super::dao::LifecycleDao;
use super::orchestrator::{Lifecycle, WallClock};
use super::reconcile::{ReconcileInput, decide};
use moatless_ports::task::PendingWorkReader;
use moatless_sandbox_runtime::convergence::{ReconcileAction, SandboxObservedStatus};
use moatless_sandbox_runtime::runtime::ErrorClass;
use moatless_types::agent::AgentProfile;
use moatless_types::sandbox::ArchiveRetention;
use moatless_types::task::CheckoutMode;

/// One durable member of a running Sandbox.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct SandboxMember {
    pub task_id: String,
    pub sandbox_id: String,
    #[sqlx(try_from = "String")]
    pub checkout_mode: CheckoutMode,
    /// The harness this Member's agent loop runs. See
    /// [`SandboxJoiner::join_task`](moatless_ports::sandbox::SandboxJoiner::join_task).
    #[sqlx(try_from = "String")]
    pub agent_profile: AgentProfile,
}

/// Delivers a pending-message wake to an already-Ready sandbox over gRPC.
///
/// A lost notification isn't rescued by a start (an already-Ready sandbox never
/// restarts), so the reconciler re-notifies until pending rows are
/// acknowledged; idempotent and fire-and-forget.
#[async_trait]
pub trait NotifyPort: Send + Sync {
    async fn notify(&self, task_id: &str);
}

/// Registers durable Sandbox members into a running sandbox process.
///
/// Replaying a member must be harmless — the sandbox-side `JoinTask` handler is
/// idempotent — so the reconciler uses it to repair pod restarts without
/// relying on process memory.
#[async_trait]
pub trait MemberRegistryPort: Send + Sync {
    async fn ensure_registered(&self, member: &SandboxMember) -> Result<(), MemberRegistryError>;
}

/// A member registration attempt failed. The reconciler logs and retries on a
/// later tick; it never creates a member context without a successful join.
#[derive(Debug, thiserror::Error)]
#[error("{context}")]
pub struct MemberRegistryError {
    pub context: String,
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync + 'static>>,
}

impl MemberRegistryError {
    pub fn new(context: impl Into<String>) -> Self {
        Self {
            context: context.into(),
            source: None,
        }
    }

    pub fn with_source(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            context: context.into(),
            source: Some(Box::new(source)),
        }
    }
}

/// How long an identical member-registration failure stays suppressed before
/// it's re-logged at `warn!`.
///
/// Without this, a permanently-failing member (retried every ~30s tick) floods
/// the log with an identical line forever.
const MEMBER_REG_RELOG_INTERVAL: Duration = Duration::from_mins(10);

/// Dedup key for a running member: `(sandbox_id, task_id)`.
type MemberKey = (String, String);

/// Per-member failure dedup state, so a persistently-failing registration warns
/// on change and periodically rather than every tick.
struct MemberFailure {
    last_error: String,
    last_warned_at: DateTime<Utc>,
    /// Identical failures hidden since the last `warn!`, surfaced in the next
    /// one so the suppression is visible rather than silent.
    suppressed_since_warn: u32,
}

/// Whether a member-registration failure should be logged at `warn!` (with the
/// count of failures hidden since the last warn) or suppressed to `debug!`.
#[derive(Debug, PartialEq, Eq)]
enum MemberFailureLog {
    Warn { suppressed: u32 },
    Suppress,
}

/// True once `relog_after` has elapsed since `last_warned_at`. An
/// unrepresentable interval (absurd config) re-logs rather than suppressing
/// forever.
fn member_relog_due(
    last_warned_at: DateTime<Utc>,
    now: DateTime<Utc>,
    relog_after: Duration,
) -> bool {
    match chrono::Duration::from_std(relog_after) {
        Ok(threshold) => now.signed_duration_since(last_warned_at) >= threshold,
        Err(_) => true,
    }
}

/// Record a member-registration failure and decide whether to warn. Warns on
/// the first failure for a member, when the error text changes, or once
/// `relog_after` has elapsed since the last warn; otherwise suppresses.
fn note_member_failure(
    failures: &mut HashMap<MemberKey, MemberFailure>,
    key: MemberKey,
    error: &str,
    now: DateTime<Utc>,
    relog_after: Duration,
) -> MemberFailureLog {
    match failures.get_mut(&key) {
        Some(prev)
            if prev.last_error == error
                && !member_relog_due(prev.last_warned_at, now, relog_after) =>
        {
            prev.suppressed_since_warn = prev.suppressed_since_warn.saturating_add(1);
            MemberFailureLog::Suppress
        }
        Some(prev) => {
            let suppressed = prev.suppressed_since_warn;
            prev.last_error = error.to_string();
            prev.last_warned_at = now;
            prev.suppressed_since_warn = 0;
            MemberFailureLog::Warn { suppressed }
        }
        None => {
            failures.insert(
                key,
                MemberFailure {
                    last_error: error.to_string(),
                    last_warned_at: now,
                    suppressed_since_warn: 0,
                },
            );
            MemberFailureLog::Warn { suppressed: 0 }
        }
    }
}

/// Flatten an error and its `source()` chain into one line.
///
/// `MemberRegistryError` carries the underlying `tonic::Status` only as
/// `#[source]`, which `Display` omits — without this, the gRPC code/message
/// that explains the failure never reaches the log.
fn format_error_chain(err: &dyn std::error::Error) -> String {
    let mut out = err.to_string();
    let mut source = err.source();
    while let Some(inner) = source {
        out.push_str(": ");
        out.push_str(&inner.to_string());
        source = inner.source();
    }
    out
}

async fn ensure_running_members_registered(
    store: &dyn LifecycleDao,
    member_registry: &dyn MemberRegistryPort,
    failures: &Mutex<HashMap<MemberKey, MemberFailure>>,
    now: DateTime<Utc>,
    relog_after: Duration,
) {
    let members = match store.running_sandbox_members().await {
        Ok(members) => members,
        Err(e) => {
            warn!(error = %e, "reconciler: running-member enumeration failed");
            return;
        }
    };

    let mut seen: HashSet<MemberKey> = HashSet::with_capacity(members.len());
    for member in &members {
        let key = (member.sandbox_id.clone(), member.task_id.clone());
        seen.insert(key.clone());
        match member_registry.ensure_registered(member).await {
            Ok(()) => {
                // Clearing on success both stops future suppression and lets a
                // recovery after prior failures be noted once.
                if failures
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .remove(&key)
                    .is_some()
                {
                    info!(
                        task_id = %member.task_id,
                        sandbox_id = %member.sandbox_id,
                        "reconciler: sandbox member registered after prior failures"
                    );
                }
            }
            Err(e) => {
                let detail = format_error_chain(&e);
                let decision = {
                    let mut guard = failures
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    note_member_failure(&mut guard, key, &detail, now, relog_after)
                };
                match decision {
                    MemberFailureLog::Warn { suppressed } => warn!(
                        task_id = %member.task_id,
                        sandbox_id = %member.sandbox_id,
                        checkout_mode = member.checkout_mode.as_str(),
                        error = %detail,
                        suppressed_repeats = suppressed,
                        "reconciler: failed to register sandbox member"
                    ),
                    MemberFailureLog::Suppress => debug!(
                        task_id = %member.task_id,
                        sandbox_id = %member.sandbox_id,
                        error = %detail,
                        "reconciler: failed to register sandbox member (repeat suppressed)"
                    ),
                }
            }
        }
    }

    // Drop dedup state for members no longer present (task removed or sandbox
    // stopped) so a later reappearance warns afresh instead of inheriting a
    // stale suppression window.
    failures
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|k, _| seen.contains(k));
}

/// Reconciler tuning.
#[derive(Debug, Clone)]
pub struct ReconcilerConfig {
    /// Attempts per operation generation before a transient failure gives up
    /// until a new caller operation or a wake.
    pub attempt_cap: u32,
    /// Grace after an intent change before the reconciler acts, so a caller's
    /// own in-flight operation converges without the reconciler racing it.
    pub grace: Duration,
    /// Exponential-backoff base and ceiling between transient retries.
    pub backoff_base: Duration,
    pub backoff_max: Duration,
}

impl Default for ReconcilerConfig {
    fn default() -> Self {
        Self {
            attempt_cap: 5,
            grace: Duration::from_secs(30),
            backoff_base: Duration::from_secs(2),
            backoff_max: Duration::from_mins(1),
        }
    }
}

/// Per-Task retry state: the leader-local budget plus the next instant the Task
/// is eligible for another attempt (exponential backoff).
struct TaskRetry {
    budget: AttemptBudget,
    next_eligible: Option<DateTime<Utc>>,
    /// The newest pending-message timestamp that has already reset this budget.
    /// A wake resets the cap only when a *newer* message arrives — the same
    /// pending row (still undelivered every tick until Ready) must not.
    last_wake_at: Option<DateTime<Utc>>,
    /// Ready re-notification backoff, keyed off the same wake advance as the
    /// operation cap.
    next_notify: Option<DateTime<Utc>>,
    notify_attempts: u32,
}

impl TaskRetry {
    fn new(cap: u32) -> Self {
        Self {
            budget: AttemptBudget::new(cap),
            next_eligible: None,
            last_wake_at: None,
            next_notify: None,
            notify_attempts: 0,
        }
    }
}

/// The desired-state reconciler.
pub struct Reconciler {
    lifecycle: Lifecycle,
    store: Arc<dyn LifecycleDao>,
    observer: Arc<dyn FleetObserver>,
    notify: Arc<dyn NotifyPort>,
    pending_work: Arc<dyn PendingWorkReader>,
    member_registry: Arc<dyn MemberRegistryPort>,
    clock: Arc<dyn WallClock>,
    retries: Mutex<HashMap<String, TaskRetry>>,
    /// Per-member registration-failure dedup, keyed by `(sandbox_id, task_id)`.
    member_reg_failures: Mutex<HashMap<MemberKey, MemberFailure>>,
    config: ReconcilerConfig,
}

pub(crate) struct ReconcilerDeps {
    pub store: Arc<dyn LifecycleDao>,
    pub observer: Arc<dyn FleetObserver>,
    pub notify: Arc<dyn NotifyPort>,
    pub pending_work: Arc<dyn PendingWorkReader>,
    pub member_registry: Arc<dyn MemberRegistryPort>,
    pub clock: Arc<dyn WallClock>,
}

impl Reconciler {
    pub(crate) fn new(
        lifecycle: Lifecycle,
        deps: ReconcilerDeps,
        config: ReconcilerConfig,
    ) -> Self {
        Self {
            lifecycle,
            store: deps.store,
            observer: deps.observer,
            notify: deps.notify,
            pending_work: deps.pending_work,
            member_registry: deps.member_registry,
            clock: deps.clock,
            retries: Mutex::new(HashMap::new()),
            member_reg_failures: Mutex::new(HashMap::new()),
            config,
        }
    }

    /// One reconcile pass over the whole fleet.
    pub async fn reconcile_once(&self) {
        let now = self.clock.now();

        // One fleet observation per tick — also a candidate source: a live
        // resource whose intent is not Running is a divergence to converge.
        let coarse = match self.observer.observe_fleet().await {
            Ok(m) => m,
            Err(e) => {
                warn!(error = %e, "reconciler: fleet observation failed; skipping tick");
                return;
            }
        };

        ensure_running_members_registered(
            self.store.as_ref(),
            self.member_registry.as_ref(),
            &self.member_reg_failures,
            now,
            MEMBER_REG_RELOG_INTERVAL,
        )
        .await;

        // One DB read yields both the candidate set and each candidate's queued
        // work, because both are columns on the same `sandboxes` row.
        let enumerated = match self.store.reconcile_candidates().await {
            Ok(c) => c,
            Err(e) => {
                warn!(error = %e, "reconciler: candidate enumeration failed");
                Vec::new()
            }
        };
        let pending_map: HashMap<String, DateTime<Utc>> = enumerated
            .iter()
            .filter_map(|c| c.pending_work_at.map(|at| (c.task_id.clone(), at)))
            .collect();
        let mut candidates: HashSet<String> = enumerated.into_iter().map(|c| c.task_id).collect();
        candidates.extend(coarse.keys().cloned());

        for task_id in &candidates {
            let pending_at = pending_map.get(task_id).copied();
            self.reconcile_task(task_id, &coarse, pending_at, now).await;
        }

        // Drop retry state for Tasks no longer in scope so the map tracks the
        // live candidate set rather than growing without bound.
        self.retries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|k, _| candidates.contains(k));
    }

    async fn reconcile_task(
        &self,
        task_id: &str,
        coarse: &HashMap<String, RuntimeObservation>,
        pending_at: Option<DateTime<Utc>>,
        now: DateTime<Utc>,
    ) {
        let mut intent = match self.store.load(task_id).await {
            Ok(Some(intent)) => intent,
            Ok(None) => return,
            Err(e) => {
                warn!(task_id, error = %e, "reconciler: intent load failed; skipping task");
                return;
            }
        };

        // Give a freshly-changed intent its grace window — a caller's own
        // detached operation is likely still converging. An oversized configured
        // grace saturates to MAX (fail closed: keep waiting) rather than
        // collapsing to no window.
        if now.signed_duration_since(intent.desired_state_updated_at)
            < chrono::Duration::from_std(self.config.grace).unwrap_or(chrono::Duration::MAX)
        {
            return;
        }

        let observed_coarse = coarse
            .get(task_id)
            .copied()
            .unwrap_or_else(|| SandboxObservedStatus::Absent.into());
        let coarse_decision = decide(&ReconcileInput {
            intent: intent.clone(),
            observed: observed_coarse.status,
            newest_pending_message_at: pending_at,
        });
        // A woken Task always confirms — even when coarse shows no operation it may
        // be an already-Ready sandbox owed a re-notification.
        if intent.marker.is_none()
            && !coarse_decision.woken
            && !coarse_decision.action.is_operation()
        {
            return;
        }

        // Confirm the divergence authoritatively before acting.
        let observed = match self
            .observer
            .observe(task_id, intent.status, intent.desired_state)
            .await
        {
            Ok(o) => o,
            Err(e) => {
                debug!(task_id, error = %e, "reconciler: confirm observation failed; skipping");
                return;
            }
        };

        if let Some(marker) = intent.marker {
            let replacement_ready = observed.status == SandboxObservedStatus::Ready
                && observed.generation == Some(marker.generation);
            let ack = moatless_sandbox_runtime::marker::acknowledge(
                &marker,
                moatless_sandbox_runtime::marker::AckContext {
                    observed: observed.status,
                    replacement_ready,
                    ..Default::default()
                },
            );
            if ack.clears() {
                if let Err(e) = self.store.clear_marker(task_id, marker.generation).await {
                    warn!(task_id, error = %e, "reconciler: failed to acknowledge converged marker");
                }
                return;
            }

            if marker.kind.is_edge() {
                if !moatless_sandbox_runtime::marker::is_stale(
                    &marker,
                    now,
                    moatless_sandbox_runtime::marker::EDGE_ABANDON_AFTER,
                ) {
                    // Caller-owned edges are never replayed. Give the fresh
                    // replacement time to appear before level recovery acts.
                    return;
                }
                if observed.status == SandboxObservedStatus::Starting
                    && observed.generation == Some(marker.generation)
                {
                    // The replacement generation is still advancing. Its age
                    // cannot by itself prove that the edge was abandoned.
                    return;
                }
                if let Err(e) = self
                    .lifecycle
                    .abandon_edge(task_id, marker.kind, marker.generation)
                    .await
                {
                    warn!(task_id, error = %e, "reconciler: failed to durably abandon stale edge");
                    return;
                }
                // The atomic abandonment commit cleared the old edge marker.
                // Any level recovery below must allocate a fresh generation.
                intent.marker = None;
            }
        }

        let decision = decide(&ReconcileInput {
            intent: intent.clone(),
            observed: observed.status,
            newest_pending_message_at: pending_at,
        });

        // A newly observed wake resets the operation cap and the notify backoff
        // (never the same pending row that stays undelivered every tick).
        if decision.woken {
            self.note_wake(task_id, pending_at);
        }

        if decision.action.is_operation() {
            if !self.retry_gate(task_id, intent.marker.map(|m| m.generation.value()), now) {
                return;
            }
            let result = match decision.action {
                ReconcileAction::Start => self.lifecycle.start(task_id).await,
                ReconcileAction::Stop => {
                    let reason = intent.stop_reason.unwrap_or(SandboxStopReason::System);
                    self.lifecycle.stop(task_id, reason).await
                }
                ReconcileAction::Remove => {
                    let retention = intent.retention.unwrap_or(ArchiveRetention::RetainArchives);
                    self.lifecycle.remove(task_id, retention).await
                }
                ReconcileAction::None | ReconcileAction::Wait => return,
            };
            if let Some(spent) = self.record_outcome(task_id, decision.action, result, now) {
                self.publish_recovery_exhausted(task_id, decision.action, spent)
                    .await;
            }
        } else if decision.woken && observed.status == SandboxObservedStatus::Ready {
            // The wake landed on an already-Ready sandbox: re-notify under the
            // per-Task backoff until the pending rows are acknowledged (which
            // drops the Task from the pending set and ends the wake).
            self.maybe_notify(task_id, now).await;
        }
    }

    /// Reset the operation cap and notify backoff when a genuinely new message
    /// arrives — the same undelivered pending row (present every tick until
    /// Ready) must not.
    fn note_wake(&self, task_id: &str, wake_at: Option<DateTime<Utc>>) {
        let Some(wake_at) = wake_at else { return };
        let mut guard = self
            .retries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let retry = guard
            .entry(task_id.to_string())
            .or_insert_with(|| TaskRetry::new(self.config.attempt_cap));
        if retry.last_wake_at.is_none_or(|prev| wake_at > prev) {
            retry.budget.reset_for_wake();
            retry.next_eligible = None;
            retry.next_notify = None;
            retry.notify_attempts = 0;
            retry.last_wake_at = Some(wake_at);
        }
    }

    /// Re-notify a Ready sandbox of its pending wake, spaced by
    /// leader-local exponential backoff keyed by Task.
    ///
    /// `task_id` is the Sandbox's oldest Task, not necessarily who queued
    /// the work: this notifies every member with an unclaimed message, or
    /// a Task that joined later would never be dispatched.
    async fn maybe_notify(&self, task_id: &str, now: DateTime<Utc>) {
        let due = {
            let mut guard = self
                .retries
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let retry = guard
                .entry(task_id.to_string())
                .or_insert_with(|| TaskRetry::new(self.config.attempt_cap));
            if retry.next_notify.is_some_and(|t| now < t) {
                false
            } else {
                retry.notify_attempts = retry.notify_attempts.saturating_add(1);
                let delay = super::attempt::backoff(
                    retry.notify_attempts,
                    self.config.backoff_base,
                    self.config.backoff_max,
                );
                retry.next_notify = Some(
                    now + chrono::Duration::from_std(delay)
                        .unwrap_or_else(|_| chrono::Duration::seconds(60)),
                );
                true
            }
        };
        if !due {
            return;
        }

        // A read failure falls back to the candidate rather than notifying
        // nobody: a wake that stalls on a transient DB error would leave the
        // turn undelivered until the next tick anyway, and the candidate is the
        // target this had before members were resolved.
        let targets = match self.pending_work.members_awaiting_dispatch(task_id).await {
            Ok(targets) if targets.is_empty() => {
                // The claim landed between the candidate read and this one.
                // Nothing is waiting, so the wake has already been answered.
                debug!(
                    task_id,
                    "reconciler: pending wake settled before notify; nothing to deliver"
                );
                return;
            }
            Ok(targets) => targets,
            Err(error) => {
                warn!(
                    task_id,
                    %error,
                    "reconciler: could not resolve the Sandbox's pending members; notifying the candidate"
                );
                vec![task_id.to_string()]
            }
        };

        for target in &targets {
            debug!(
                task_id,
                target, "reconciler: re-notifying Ready sandbox of pending wake"
            );
            self.notify.notify(target).await;
        }
    }

    /// Decide whether to drive an operation this tick under the attempt cap +
    /// backoff. Mutates the Task's budget (`begin`) before gating. Wake resets
    /// are handled by [`note_wake`](Self::note_wake) before this runs.
    fn retry_gate(&self, task_id: &str, generation: Option<i64>, now: DateTime<Utc>) -> bool {
        let mut guard = self
            .retries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let retry = guard
            .entry(task_id.to_string())
            .or_insert_with(|| TaskRetry::new(self.config.attempt_cap));

        if let Some(g) = generation {
            retry
                .budget
                .begin(moatless_sandbox_runtime::operation::OperationGeneration(g));
        }

        if let Some(t) = retry.next_eligible {
            if now < t {
                return false;
            }
        }
        // A tracked generation whose cap is exhausted (or hit a permanent
        // failure) waits for a new caller operation or a wake. A markerless
        // first attempt has no generation to key on and is always allowed:
        // it establishes the marker the next tick keys against.
        if generation.is_some() && retry.budget.exhausted() {
            return false;
        }
        true
    }

    /// Returns what the give-up event needs only when this call is the one that
    /// spends the budget, so the caller publishes `recovery_exhausted` exactly
    /// once per generation; `None` otherwise, including on an already-spent
    /// budget.
    fn record_outcome(
        &self,
        task_id: &str,
        action: ReconcileAction,
        result: Result<
            moatless_sandbox_runtime::outcome::LifecycleOutcome,
            super::error::LifecycleError,
        >,
        now: DateTime<Utc>,
    ) -> Option<SpentBudget> {
        let mut guard = self
            .retries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let retry = guard
            .entry(task_id.to_string())
            .or_insert_with(|| TaskRetry::new(self.config.attempt_cap));

        match result {
            Ok(_) => {
                retry.next_eligible = None;
                debug!(task_id, ?action, "reconciler: drove lifecycle operation");
                None
            }
            Err(e) => {
                let was_exhausted = retry.budget.exhausted();
                match e.class() {
                    ErrorClass::Transient => {
                        retry.budget.record_failure();
                        let delay = retry
                            .budget
                            .backoff(self.config.backoff_base, self.config.backoff_max);
                        retry.next_eligible = Some(
                            now + chrono::Duration::from_std(delay)
                                .unwrap_or_else(|_| chrono::Duration::seconds(60)),
                        );
                        warn!(task_id, ?action, error = %e, "reconciler: transient failure; will retry");
                    }
                    // Another writer holds the per-Task lock, driving this
                    // same generation. The budget stays untouched so the
                    // next tick finds the lock free — spending it here
                    // would report `recovery_exhausted` for work already
                    // in progress, then refuse to retry it.
                    ErrorClass::Contention => {
                        debug!(task_id, ?action, error = %e, "reconciler: lock contention; retrying next tick");
                        return None;
                    }
                    ErrorClass::Permanent => {
                        retry.budget.block();
                        info!(task_id, ?action, error = %e, "reconciler: permanent failure; not retrying this generation");
                    }
                }
                (!was_exhausted && retry.budget.exhausted()).then(|| SpentBudget {
                    attempts: retry.budget.attempts(),
                    last_error: e.to_string(),
                })
            }
        }
    }

    /// Without it a spent Task shows one deduped `error` then silence forever —
    /// unreadable as "still retrying" vs "never coming back". Scoped to the
    /// marker's generation, so a first attempt that spends the budget before
    /// any marker exists has nothing to key on and is left to the logs.
    async fn publish_recovery_exhausted(
        &self,
        task_id: &str,
        action: ReconcileAction,
        spent: SpentBudget,
    ) {
        let marker = match self.store.load(task_id).await {
            Ok(Some(intent)) => intent.marker,
            Ok(None) => None,
            Err(e) => {
                warn!(task_id, error = %e, "reconciler: intent load failed; cannot publish recovery_exhausted");
                return;
            }
        };
        let Some(marker) = marker else {
            warn!(
                task_id,
                ?action,
                "reconciler: recovery exhausted with no marker; not publishing"
            );
            return;
        };
        self.lifecycle
            .publish_recovery_exhausted(
                task_id,
                marker.kind,
                marker.generation,
                spent.attempts,
                &spent.last_error,
            )
            .await;
    }
}

/// What the terminal `recovery_exhausted` fact reports: the attempts actually
/// spent on this generation, and the failure that spent the last of them.
struct SpentBudget {
    attempts: u32,
    last_error: String,
}

/// Run reconciliation on elected ticks until the host cancels it.
pub fn spawn(
    reconciler: Reconciler,
    leader: Option<Arc<dyn moatless_core::leader::LeaderGuard>>,
    cancel: tokio_util::sync::CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(30));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                _ = ticker.tick() => {
                    if let Some(l) = &leader {
                        match l.try_acquire_or_renew().await {
                            Ok(true) => {}
                            Ok(false) => continue,
                            Err(e) => {
                                tracing::warn!(error = %e, "reconciler: leader check failed; skipping tick");
                                continue;
                            }
                        }
                    }
                    reconciler.reconcile_once().await;
                }
            }
        }
        info!("Sandbox reconciler stopped");
    });
    info!("Sandbox reconciler spawned");
    handle
}

#[cfg(test)]
mod tests;

#[cfg(all(test, feature = "db-tests"))]
mod db_tests;
