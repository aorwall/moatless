//! The desired-state reconciler — the single async recovery owner for level
//! states.
//!
//! One tick: enumerate DB candidates (desired-Running participants + incomplete
//! markers) ∪ pending-message wakes ∪ Tasks the fleet observation shows as live,
//! observe the fleet once, and for each candidate compare intent against
//! observed via [`reconcile::decide`](super::reconcile::decide). A divergence is
//! confirmed with an authoritative per-Task read, then driven through a lifecycle
//! operation — never a direct runtime mutation. Retries are bounded by a
//! leader-local [`AttemptBudget`] with exponential backoff; permanent failures
//! and Closed are never retried.
//!
//! The reconciler holds no lease logic itself: the loop that drives
//! [`Reconciler::reconcile_once`] is lease-elected on Kubernetes and lease-free
//! single-replica on Docker.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use tracing::{debug, info, warn};

use moatless_types::sandbox::SandboxStopReason;

use moatless_sandbox_runtime::fleet::{FleetObserver, RuntimeObservation};

use super::attempt::AttemptBudget;
use super::orchestrator::{Lifecycle, LifecycleDao, SandboxMember, WallClock};
use super::reconcile::{ReconcileInput, decide};
use moatless_ports::task::PendingWorkReader;
use moatless_sandbox_runtime::convergence::{ReconcileAction, SandboxObservedStatus};
use moatless_sandbox_runtime::runtime::ErrorClass;
use moatless_types::sandbox::ArchiveRetention;

/// Delivers a pending-message wake to an already-Ready sandbox over gRPC.
///
/// A wake whose gRPC notification is lost while the sandbox is already Ready is
/// not rescued by a start (the sandbox never restarts), so the reconciler
/// re-notifies a woken Ready Task until its pending rows are acknowledged.
/// Idempotent and fire-and-forget.
#[async_trait]
pub trait NotifyPort: Send + Sync {
    async fn notify(&self, task_id: &str);
}

/// Registers durable Sandbox members into a running sandbox process.
///
/// Replaying a registered member must be harmless: the sandbox-side
/// `JoinTask` handler is idempotent, and the reconciler uses that property to
/// repair pod restarts without relying on process memory.
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

/// How long an identical member-registration failure stays suppressed before it
/// is re-logged at `warn!`. The reconciler re-attempts every member on every
/// tick (~30s); without this, one permanently-failing member floods the log
/// with an identical line every tick forever.
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

/// True once `relog_after` has elapsed since `last_warned_at`. An unrepresentable
/// interval (absurd config) re-logs rather than suppressing forever.
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

/// Record a member-registration failure and decide whether to warn. Warns on the
/// first failure for a member, when the error text changes, or once
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

/// Flatten an error and its `source()` chain into one line. `MemberRegistryError`
/// carries the underlying `tonic::Status` (via `classify_grpc_error`) only as a
/// `#[source]`, which `Display` omits — so the gRPC code/message that actually
/// explains the failure would otherwise never reach the log.
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
    /// Grace after an intent change before the reconciler acts, so a caller's own
    /// in-flight operation converges without the reconciler racing it.
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

pub struct ReconcilerDeps {
    pub store: Arc<dyn LifecycleDao>,
    pub observer: Arc<dyn FleetObserver>,
    pub notify: Arc<dyn NotifyPort>,
    pub pending_work: Arc<dyn PendingWorkReader>,
    pub member_registry: Arc<dyn MemberRegistryPort>,
    pub clock: Arc<dyn WallClock>,
}

impl Reconciler {
    pub fn new(lifecycle: Lifecycle, deps: ReconcilerDeps, config: ReconcilerConfig) -> Self {
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

    /// Re-notify a Ready sandbox of its pending wake, spaced by leader-local
    /// exponential backoff keyed by Task.
    ///
    /// `task_id` is the Sandbox's candidate, which is a handle on the Sandbox
    /// and not necessarily the Task the work is queued for: several Tasks can
    /// share one Sandbox, and the candidate is the oldest of them. The pod
    /// dispatches per Task, so the wake is delivered to each member holding an
    /// unclaimed message. Notifying the candidate alone would leave a Task that
    /// joined an existing Sandbox undispatched forever, since its message never
    /// clears the wake and the wake never reaches it.
    ///
    /// The backoff stays keyed on the candidate: the wake is one per Sandbox,
    /// so the whole fan-out shares its spacing, and `retries` keeps tracking the
    /// candidate set that `reconcile_once` retains.
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

        // Backing off between transient retries.
        if let Some(t) = retry.next_eligible {
            if now < t {
                return false;
            }
        }
        // A tracked generation whose cap is exhausted (or blocked by a permanent
        // failure) waits for a new caller operation or a wake. A markerless first
        // attempt has no generation to key on and is always allowed — it
        // establishes the marker the next tick keys against.
        if generation.is_some() && retry.budget.exhausted() {
            return false;
        }
        true
    }

    /// Record one operation's result against the Task's budget.
    ///
    /// Returns what the give-up event needs when this call is the one that
    /// **spends** the budget, so the caller publishes the terminal
    /// `recovery_exhausted` fact exactly once per generation. Returns `None`
    /// otherwise, including on a budget that was already spent.
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
                    // Another writer holds the per-Task lock and is driving this
                    // same generation. The budget stays untouched so the next
                    // tick finds the lock free: spending it here published a
                    // `recovery_exhausted` error for work already in progress,
                    // and then refused to drive the operation again.
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

    /// Publish the terminal give-up fact. Without it, a Task whose recovery is
    /// spent shows one deduped `error` and then silence forever — the reader
    /// cannot tell "still retrying" from "never coming back on its own".
    ///
    /// Scoped to the marker's generation, so it needs one: a first attempt that
    /// spends the budget before any marker is persisted has nothing stable to
    /// key on and is left to the logs.
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
mod tests {
    use super::*;

    use moatless_types::task::CheckoutMode;

    use super::super::events::LifecycleEvent;
    use super::super::orchestrator::{IntentWrite, StoreError, TaskIntent};
    use moatless_sandbox_runtime::operation::{IntentEffect, OperationGeneration};

    struct MemberStore {
        members: Result<Vec<SandboxMember>, StoreError>,
    }

    #[async_trait]
    impl LifecycleDao for MemberStore {
        async fn load(&self, _task_id: &str) -> Result<Option<TaskIntent>, StoreError> {
            panic!("member registration test must not load task intent")
        }

        async fn reconcile_candidates(
            &self,
        ) -> Result<Vec<crate::lifecycle::orchestrator::ReconcileCandidate>, StoreError> {
            panic!("member registration test must not enumerate reconcile candidates")
        }

        async fn running_sandbox_members(&self) -> Result<Vec<SandboxMember>, StoreError> {
            match &self.members {
                Ok(members) => Ok(members.clone()),
                Err(e) => Err(StoreError::new(e.to_string())),
            }
        }

        async fn sandbox_shape(
            &self,
            _sandbox_id: &str,
        ) -> Result<Option<crate::lifecycle::orchestrator::SandboxShape>, StoreError> {
            panic!("member registration test must not read a sandbox shape")
        }

        async fn has_other_active_members(
            &self,
            _sandbox_id: &str,
            _task_id: &str,
        ) -> Result<bool, StoreError> {
            panic!("member registration test must not enumerate active peers")
        }

        async fn allocate_generation(
            &self,
            _task_id: &str,
        ) -> Result<OperationGeneration, StoreError> {
            panic!("member registration test must not allocate generations")
        }

        async fn persist_intent(
            &self,
            _task_id: &str,
            _write: &IntentWrite,
        ) -> Result<IntentEffect, StoreError> {
            panic!("member registration test must not persist intent")
        }

        async fn commit_fact_and_clear_marker(
            &self,
            _sandbox_id: &str,
            _task_id: &str,
            _event: &LifecycleEvent,
        ) -> Result<(), StoreError> {
            panic!("member registration test must not commit lifecycle facts")
        }

        async fn clear_marker(
            &self,
            _task_id: &str,
            _generation: OperationGeneration,
        ) -> Result<(), StoreError> {
            panic!("member registration test must not clear markers")
        }

        async fn stopped_with_storage(
            &self,
            _task_ids: &[String],
        ) -> Result<Vec<(String, DateTime<Utc>)>, StoreError> {
            panic!("member registration test must not enumerate stopped storage")
        }

        async fn mark_storage_reclaimed(&self, _task_id: &str) -> Result<(), StoreError> {
            panic!("member registration test must not mark storage reclaimed")
        }
        async fn record_running_commands(
            &self,
            _task_id: &str,
            _running: i32,
        ) -> Result<(), StoreError> {
            panic!("member registration test must not record running commands")
        }
    }

    #[derive(Default)]
    struct RecordingRegistry {
        calls: Mutex<Vec<SandboxMember>>,
    }

    #[async_trait]
    impl MemberRegistryPort for RecordingRegistry {
        async fn ensure_registered(
            &self,
            member: &SandboxMember,
        ) -> Result<(), MemberRegistryError> {
            self.calls.lock().unwrap().push(member.clone());
            Ok(())
        }
    }

    /// Fails every registration with a fixed `MemberRegistryError` carrying a
    /// `source`, exercising the failure/dedup path and the source-chain log.
    struct FailingRegistry {
        context: String,
        source: String,
    }

    #[async_trait]
    impl MemberRegistryPort for FailingRegistry {
        async fn ensure_registered(
            &self,
            _member: &SandboxMember,
        ) -> Result<(), MemberRegistryError> {
            Err(MemberRegistryError::with_source(
                self.context.clone(),
                std::io::Error::other(self.source.clone()),
            ))
        }
    }

    fn member(task_id: &str, sandbox_id: &str) -> SandboxMember {
        SandboxMember {
            task_id: task_id.to_string(),
            sandbox_id: sandbox_id.to_string(),
            checkout_mode: CheckoutMode::Shared,
        }
    }

    fn empty_failures() -> Mutex<HashMap<MemberKey, MemberFailure>> {
        Mutex::new(HashMap::new())
    }

    fn t0() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }

    #[tokio::test]
    async fn running_members_are_registered_from_durable_store() {
        let members = vec![member("lead", "sandbox-1"), member("guest", "sandbox-1")];
        let store = MemberStore {
            members: Ok(members.clone()),
        };
        let registry = RecordingRegistry::default();

        ensure_running_members_registered(
            &store,
            &registry,
            &empty_failures(),
            t0(),
            MEMBER_REG_RELOG_INTERVAL,
        )
        .await;

        assert_eq!(*registry.calls.lock().unwrap(), members);
    }

    #[tokio::test]
    async fn member_enumeration_failure_registers_nothing() {
        let store = MemberStore {
            members: Err(StoreError::new("database unavailable")),
        };
        let registry = RecordingRegistry::default();

        ensure_running_members_registered(
            &store,
            &registry,
            &empty_failures(),
            t0(),
            MEMBER_REG_RELOG_INTERVAL,
        )
        .await;

        assert!(registry.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn first_failure_warns_then_identical_repeat_suppresses_within_window() {
        let mut failures = HashMap::new();
        let key = ("sandbox-1".to_string(), "task-1".to_string());

        let first = note_member_failure(
            &mut failures,
            key.clone(),
            "JoinTask failed: unavailable",
            t0(),
            MEMBER_REG_RELOG_INTERVAL,
        );
        assert_eq!(first, MemberFailureLog::Warn { suppressed: 0 });

        let repeat = note_member_failure(
            &mut failures,
            key.clone(),
            "JoinTask failed: unavailable",
            t0() + chrono::Duration::seconds(30),
            MEMBER_REG_RELOG_INTERVAL,
        );
        assert_eq!(repeat, MemberFailureLog::Suppress);
        assert_eq!(failures.get(&key).unwrap().suppressed_since_warn, 1);
    }

    #[test]
    fn changed_error_warns_immediately_and_reports_suppressed_count() {
        let mut failures = HashMap::new();
        let key = ("sandbox-1".to_string(), "task-1".to_string());

        note_member_failure(
            &mut failures,
            key.clone(),
            "err-a",
            t0(),
            MEMBER_REG_RELOG_INTERVAL,
        );
        // Two identical repeats are hidden.
        note_member_failure(
            &mut failures,
            key.clone(),
            "err-a",
            t0() + chrono::Duration::seconds(30),
            MEMBER_REG_RELOG_INTERVAL,
        );
        note_member_failure(
            &mut failures,
            key.clone(),
            "err-a",
            t0() + chrono::Duration::seconds(60),
            MEMBER_REG_RELOG_INTERVAL,
        );

        let changed = note_member_failure(
            &mut failures,
            key.clone(),
            "err-b",
            t0() + chrono::Duration::seconds(90),
            MEMBER_REG_RELOG_INTERVAL,
        );
        assert_eq!(changed, MemberFailureLog::Warn { suppressed: 2 });
        assert_eq!(failures.get(&key).unwrap().last_error, "err-b");
        assert_eq!(failures.get(&key).unwrap().suppressed_since_warn, 0);
    }

    #[test]
    fn identical_failure_rewarns_after_relog_interval() {
        let mut failures = HashMap::new();
        let key = ("sandbox-1".to_string(), "task-1".to_string());

        note_member_failure(
            &mut failures,
            key.clone(),
            "err",
            t0(),
            MEMBER_REG_RELOG_INTERVAL,
        );
        let after = note_member_failure(
            &mut failures,
            key.clone(),
            "err",
            t0() + chrono::Duration::from_std(MEMBER_REG_RELOG_INTERVAL).unwrap(),
            MEMBER_REG_RELOG_INTERVAL,
        );
        assert!(matches!(after, MemberFailureLog::Warn { .. }));
    }

    #[tokio::test]
    async fn repeated_registration_failure_accumulates_suppressed_count() {
        let members = vec![member("task-1", "sandbox-1")];
        let store = MemberStore {
            members: Ok(members),
        };
        let registry = FailingRegistry {
            context: "JoinTask failed".to_string(),
            source: "status: FailedPrecondition, message: clone failed".to_string(),
        };
        let failures = empty_failures();

        ensure_running_members_registered(
            &store,
            &registry,
            &failures,
            t0(),
            MEMBER_REG_RELOG_INTERVAL,
        )
        .await;
        ensure_running_members_registered(
            &store,
            &registry,
            &failures,
            t0() + chrono::Duration::seconds(30),
            MEMBER_REG_RELOG_INTERVAL,
        )
        .await;

        let key = ("sandbox-1".to_string(), "task-1".to_string());
        let guard = failures.lock().unwrap();
        let entry = guard.get(&key).expect("failure recorded");
        assert_eq!(entry.suppressed_since_warn, 1);
        // The source chain is captured, not just the bare context.
        assert!(
            entry.last_error.contains("clone failed"),
            "chain: {}",
            entry.last_error
        );
    }

    #[tokio::test]
    async fn success_clears_failure_state_and_absent_members_are_pruned() {
        // Seed a stale failure for a member that will not appear this tick, plus
        // a member that registers successfully.
        let failures = empty_failures();
        failures.lock().unwrap().insert(
            ("gone-sandbox".to_string(), "gone-task".to_string()),
            MemberFailure {
                last_error: "stale".to_string(),
                last_warned_at: t0(),
                suppressed_since_warn: 3,
            },
        );
        failures.lock().unwrap().insert(
            ("sandbox-1".to_string(), "task-1".to_string()),
            MemberFailure {
                last_error: "was failing".to_string(),
                last_warned_at: t0(),
                suppressed_since_warn: 1,
            },
        );

        let store = MemberStore {
            members: Ok(vec![member("task-1", "sandbox-1")]),
        };
        let registry = RecordingRegistry::default();

        ensure_running_members_registered(
            &store,
            &registry,
            &failures,
            t0() + chrono::Duration::seconds(30),
            MEMBER_REG_RELOG_INTERVAL,
        )
        .await;

        // Successful member cleared, absent member pruned → map empty.
        assert!(failures.lock().unwrap().is_empty());
    }

    #[test]
    fn format_error_chain_includes_source() {
        let err = MemberRegistryError::with_source(
            "JoinTask failed",
            std::io::Error::other("status: Unimplemented"),
        );
        assert_eq!(
            format_error_chain(&err),
            "JoinTask failed: status: Unimplemented"
        );
    }
}

#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::*;

    use sqlx::PgPool;

    use super::super::dao::PgLifecycleDao;
    use super::super::error::LifecycleError;
    use super::super::events::LifecycleEvent;
    use super::super::lock::PgLifecycleLock;
    use super::super::orchestrator::{
        AgentStopPort, IntentWrite, LifecycleEventSink, SystemWallClock,
    };
    use super::super::test_support::{seed_bare_task, seed_task_with as seed};
    use moatless_sandbox_runtime::fleet::ObserveError;
    use moatless_sandbox_runtime::observation_events::EventFact;
    use moatless_sandbox_runtime::operation::{
        CorrelationMarker, ObservedStatus, OperationGeneration, OperationKind,
    };
    use moatless_sandbox_runtime::runtime::{
        RuntimeOperationContext, RuntimeOutcome, SandboxRuntime,
    };
    use moatless_types::sandbox::SandboxDesiredState;
    use moatless_types::task::TaskStatus;

    // ---- Recording ports ----

    #[derive(Default)]
    struct RuntimeCalls {
        ensure: u32,
        stop: u32,
        remove: u32,
    }

    struct RecordingRuntime {
        calls: Mutex<RuntimeCalls>,
        /// When set, every `ensure_running` returns this transient failure.
        fail_ensure: bool,
    }

    impl RecordingRuntime {
        fn new() -> Self {
            Self {
                calls: Mutex::new(RuntimeCalls::default()),
                fail_ensure: false,
            }
        }
        fn failing_ensure() -> Self {
            Self {
                calls: Mutex::new(RuntimeCalls::default()),
                fail_ensure: true,
            }
        }
        fn calls(&self) -> (u32, u32, u32) {
            let c = self.calls.lock().unwrap();
            (c.ensure, c.stop, c.remove)
        }
    }

    #[async_trait]
    impl SandboxRuntime for RecordingRuntime {
        async fn ensure_running(
            &self,
            _t: &str,
            _c: RuntimeOperationContext,
        ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
            self.calls.lock().unwrap().ensure += 1;
            if self.fail_ensure {
                return Err(
                    moatless_sandbox_runtime::runtime::RuntimeError::Infrastructure("boom".into()),
                );
            }
            Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
        }
        async fn stop(
            &self,
            _t: &str,
            _c: RuntimeOperationContext,
        ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
            self.calls.lock().unwrap().stop += 1;
            Ok(RuntimeOutcome::applied(ObservedStatus::Stopped))
        }
        async fn restart(
            &self,
            _t: &str,
            _c: RuntimeOperationContext,
        ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
            Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
        }
        async fn redeploy(
            &self,
            _t: &str,
            _c: RuntimeOperationContext,
        ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
            Ok(RuntimeOutcome::applied(ObservedStatus::Starting))
        }
        async fn remove(
            &self,
            _t: &str,
            _r: ArchiveRetention,
            _c: RuntimeOperationContext,
        ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
            self.calls.lock().unwrap().remove += 1;
            Ok(RuntimeOutcome::applied(ObservedStatus::Removed))
        }
        async fn reclaim_storage(
            &self,
            _t: &str,
            _rescue_upload_url: Option<&str>,
            _c: RuntimeOperationContext,
        ) -> Result<RuntimeOutcome, moatless_sandbox_runtime::runtime::RuntimeError> {
            // The reconciler never drives the cold transition — it converges
            // desired vs. actual, and a reclaimed Task's desired state does not
            // change.
            Ok(RuntimeOutcome::no_op(ObservedStatus::Stopped))
        }
    }

    #[derive(Default)]
    struct CollectSink {
        facts: Mutex<Vec<EventFact>>,
        published: Mutex<Vec<LifecycleEvent>>,
    }
    #[async_trait]
    impl LifecycleEventSink for CollectSink {
        async fn publish(&self, _t: &str, event: &LifecycleEvent) {
            self.facts.lock().unwrap().push(event.fact);
            self.published.lock().unwrap().push(event.clone());
        }

        async fn publish_diagnostics(
            &self,
            _task_id: &str,
            _generation: OperationGeneration,
            _observed: ObservedStatus,
            _diagnostics: &moatless_sandbox_runtime::outcome::Diagnostics,
        ) {
        }
    }

    struct NoopAgentStop;
    #[async_trait]
    impl AgentStopPort for NoopAgentStop {
        async fn stop_agent(&self, _t: &str, _reason: SandboxStopReason) {}
    }

    #[derive(Default)]
    struct RecordingNotify {
        count: Mutex<u32>,
        targets: Mutex<Vec<String>>,
    }
    #[async_trait]
    impl NotifyPort for RecordingNotify {
        async fn notify(&self, t: &str) {
            *self.count.lock().unwrap() += 1;
            self.targets.lock().unwrap().push(t.to_string());
        }
    }

    /// The production `PendingWorkReader` predicate, run against the test pool.
    ///
    /// A stub returning a fixed list would not exercise what the fan-out is
    /// for, which is resolving the members of a shared Sandbox from the rows a
    /// test actually seeded. The statement matches
    /// `moatless_tasks::message::dao::members_awaiting_dispatch`; that crate is
    /// not a dependency here, and this is test code, so it is restated rather
    /// than shared.
    struct PoolPendingWork(PgPool);
    #[async_trait]
    impl PendingWorkReader for PoolPendingWork {
        async fn members_awaiting_dispatch(
            &self,
            task_id: &str,
        ) -> Result<Vec<String>, moatless_types::store::StoreError> {
            sqlx::query_scalar(
                r#"SELECT peer.task_id
                     FROM tasks t
                     JOIN tasks peer ON peer.sandbox_id = t.sandbox_id
                    WHERE t.task_id = $1
                      AND peer.status IN ('open', 'error')
                      AND EXISTS (
                          SELECT 1
                            FROM task_messages m
                           WHERE m.task_id = peer.task_id
                             AND m.agent_session_id IS NULL
                             AND m.message_type = 'user'
                      )
                    ORDER BY peer.created_at_db NULLS LAST, peer.task_id"#,
            )
            .bind(task_id)
            .fetch_all(&self.0)
            .await
            .map_err(|e| moatless_types::store::StoreError::with_source("pending", e))
        }
    }

    #[derive(Default)]
    struct RecordingMemberRegistry {
        calls: Mutex<Vec<SandboxMember>>,
    }
    #[async_trait]
    impl MemberRegistryPort for RecordingMemberRegistry {
        async fn ensure_registered(
            &self,
            member: &SandboxMember,
        ) -> Result<(), MemberRegistryError> {
            self.calls.lock().unwrap().push(member.clone());
            Ok(())
        }
    }

    /// A fixed fleet + confirm status the whole tick sees.
    struct FixedObserver {
        fleet: HashMap<String, RuntimeObservation>,
        confirm: RuntimeObservation,
    }
    #[async_trait]
    impl FleetObserver for FixedObserver {
        async fn observe_fleet(&self) -> Result<HashMap<String, RuntimeObservation>, ObserveError> {
            Ok(self.fleet.clone())
        }
        async fn observe(
            &self,
            _task_id: &str,
            _task_status: TaskStatus,
            _desired_state: SandboxDesiredState,
        ) -> Result<RuntimeObservation, ObserveError> {
            Ok(self.confirm)
        }
    }

    /// A clock that jumps forward a fixed step on every read, so successive
    /// reconcile ticks always land past the exponential backoff window.
    struct StepClock {
        base: DateTime<Utc>,
        step: chrono::Duration,
        reads: Mutex<i32>,
    }
    impl WallClock for StepClock {
        fn now(&self) -> DateTime<Utc> {
            let mut r = self.reads.lock().unwrap();
            let t = self.base + self.step * *r;
            *r += 1;
            t
        }
    }

    fn build(
        pool: PgPool,
        lock_pool: PgPool,
        runtime: Arc<RecordingRuntime>,
        observer: Arc<dyn FleetObserver>,
        grace: Duration,
    ) -> (Reconciler, Arc<CollectSink>) {
        build_with(
            pool,
            lock_pool,
            runtime,
            observer,
            Arc::new(RecordingNotify::default()),
            Arc::new(RecordingMemberRegistry::default()),
            Arc::new(SystemWallClock),
            ReconcilerConfig {
                grace,
                ..Default::default()
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn build_with(
        pool: PgPool,
        lock_pool: PgPool,
        runtime: Arc<RecordingRuntime>,
        observer: Arc<dyn FleetObserver>,
        notify: Arc<dyn NotifyPort>,
        member_registry: Arc<dyn MemberRegistryPort>,
        clock: Arc<dyn WallClock>,
        config: ReconcilerConfig,
    ) -> (Reconciler, Arc<CollectSink>) {
        let store = Arc::new(PgLifecycleDao::new(pool.clone()));
        let sink = Arc::new(CollectSink::default());
        let lifecycle = Lifecycle::new(
            store.clone(),
            runtime,
            Arc::new(NoopAgentStop),
            sink.clone(),
            Arc::new(PgLifecycleLock::new(lock_pool)),
            Arc::new(SystemWallClock),
        );
        let reconciler = Reconciler::new(
            lifecycle,
            ReconcilerDeps {
                store,
                observer,
                notify,
                pending_work: Arc::new(PoolPendingWork(pool)),
                member_registry,
                clock,
            },
            config,
        );
        (reconciler, sink)
    }

    /// A pending user Message, plus the Sandbox wake signal its send path
    /// writes. The two are one transaction in production
    /// (`moatless_tasks::message::dao::PgMessageDao::append`); the fixture
    /// records both so the reconciler sees the shape it reads.
    async fn seed_pending_user_message(pool: &PgPool, task_id: &str, created_at: DateTime<Utc>) {
        sqlx::query(
            r#"INSERT INTO task_messages
                   (task_id, uuid, message_type, user_type, timestamp, message_role,
                    message_text, created_at)
               VALUES ($1, $2, 'user', 'human', $3, 'user', '', $4)"#,
        )
        .bind(task_id)
        .bind(format!("{task_id}-msg"))
        .bind(created_at.to_rfc3339())
        .bind(created_at)
        .execute(pool)
        .await
        .expect("seed pending message");

        sqlx::query(
            r#"UPDATE sandboxes s
                  SET pending_work_at = GREATEST(s.pending_work_at, $2)
                 FROM tasks t
                WHERE t.task_id = $1
                  AND s.sandbox_id = t.sandbox_id"#,
        )
        .bind(task_id)
        .bind(created_at)
        .execute(pool)
        .await
        .expect("queue sandbox wake signal");
    }

    fn observer(
        fleet: &[(&str, SandboxObservedStatus)],
        confirm: SandboxObservedStatus,
    ) -> Arc<dyn FleetObserver> {
        Arc::new(FixedObserver {
            fleet: fleet
                .iter()
                .map(|(k, v)| (k.to_string(), (*v).into()))
                .collect(),
            confirm: confirm.into(),
        })
    }

    fn observer_with_generation(
        task_id: &str,
        status: SandboxObservedStatus,
        generation: Option<OperationGeneration>,
    ) -> Arc<dyn FleetObserver> {
        let observation = RuntimeObservation::new(status, generation);
        Arc::new(FixedObserver {
            fleet: HashMap::from([(task_id.to_string(), observation)]),
            confirm: observation,
        })
    }

    #[sqlx::test]
    async fn starts_a_desired_running_task_observed_absent(pool: PgPool) {
        seed(&pool, "t1", "open", "running").await;
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, sink) = build(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(&[], SandboxObservedStatus::Absent),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        assert_eq!(runtime.calls().0, 1, "ensure_running driven once");
        assert!(
            sink.facts
                .lock()
                .unwrap()
                .contains(&EventFact::Initializing)
        );
    }

    #[sqlx::test]
    async fn wakes_a_stopped_task_with_a_newer_pending_message(pool: PgPool) {
        seed(&pool, "t1", "open", "stopped").await;
        // A message strictly newer than the stop supersedes it.
        seed_pending_user_message(&pool, "t1", Utc::now() + chrono::Duration::hours(1)).await;
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, _sink) = build(
            pool.clone(),
            pool.clone(),
            runtime.clone(),
            observer(&[], SandboxObservedStatus::Absent),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        assert_eq!(runtime.calls().0, 1, "wake drove a start");
        let desired: String =
            sqlx::query_scalar("SELECT desired_state FROM sandboxes WHERE sandbox_id = 't1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(desired, "running", "wake flipped intent to running");
    }

    #[sqlx::test]
    async fn pending_message_wakes_a_never_operated_sandbox(pool: PgPool) {
        seed_bare_task(&pool, "t1", "open").await;
        seed_pending_user_message(&pool, "t1", Utc::now()).await;
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, _sink) = build(
            pool.clone(),
            pool.clone(),
            runtime.clone(),
            observer(&[], SandboxObservedStatus::Absent),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        assert_eq!(runtime.calls().0, 1, "durable pending Message drove start");
        let desired: String =
            sqlx::query_scalar("SELECT desired_state FROM sandboxes WHERE sandbox_id = 't1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(desired, "running");
    }

    #[sqlx::test]
    async fn a_persistent_wake_still_honours_the_attempt_cap(pool: PgPool) {
        // One pending message supersedes the stop, but the start keeps failing
        // transiently. The same undelivered row must not reset the budget every
        // tick — retries stay bounded by the cap instead of running forever.
        seed(&pool, "t1", "open", "stopped").await;
        seed_pending_user_message(&pool, "t1", Utc::now() + chrono::Duration::hours(1)).await;
        let runtime = Arc::new(RecordingRuntime::failing_ensure());
        // Each tick jumps an hour so the exponential backoff never gates —
        // isolating the wake-reset behaviour as the only thing bounding retries.
        let clock = Arc::new(StepClock {
            base: DateTime::from_timestamp(2_000_000_000, 0).unwrap(),
            step: chrono::Duration::hours(1),
            reads: Mutex::new(0),
        });
        let (rec, _sink) = build_with(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(&[], SandboxObservedStatus::Absent),
            Arc::new(RecordingNotify::default()),
            Arc::new(RecordingMemberRegistry::default()),
            clock,
            ReconcilerConfig {
                grace: Duration::ZERO,
                attempt_cap: 3,
                ..Default::default()
            },
        );

        for _ in 0..8 {
            rec.reconcile_once().await;
        }

        // Bounded near the cap. Without the newest-message guard the wake would
        // reset every tick and drive all 8.
        let ensure = runtime.calls().0;
        assert!(
            ensure <= 5,
            "ensure driven {ensure} times; the cap should bound it"
        );
        assert!(
            ensure >= 3,
            "ensure should retry up to the cap, got {ensure}"
        );
    }

    #[sqlx::test]
    async fn spending_the_attempt_cap_publishes_one_recovery_exhausted_fact(pool: PgPool) {
        // Desired Running, nothing running, every start failing: the retries all
        // reuse the generation, so their `Error` facts dedup to one id and the
        // give-up is otherwise indistinguishable from silence. The terminal fact
        // is what makes "not coming back on its own" readable.
        seed(&pool, "t1", "open", "running").await;
        let runtime = Arc::new(RecordingRuntime::failing_ensure());
        let clock = Arc::new(StepClock {
            base: DateTime::from_timestamp(2_000_000_000, 0).unwrap(),
            step: chrono::Duration::hours(1),
            reads: Mutex::new(0),
        });
        let (rec, sink) = build_with(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(&[], SandboxObservedStatus::Absent),
            Arc::new(RecordingNotify::default()),
            Arc::new(RecordingMemberRegistry::default()),
            clock,
            ReconcilerConfig {
                grace: Duration::ZERO,
                attempt_cap: 2,
                ..Default::default()
            },
        );

        for _ in 0..6 {
            rec.reconcile_once().await;
        }

        let published = sink.published.lock().unwrap().clone();
        let exhausted: Vec<_> = published
            .iter()
            .filter(|e| e.fact == EventFact::RecoveryExhausted)
            .collect();
        assert_eq!(
            exhausted.len(),
            1,
            "exactly one terminal fact per spent generation, got {}",
            exhausted.len()
        );
        let payload = exhausted[0].payload();
        assert_eq!(
            payload.lifecycle_event.as_deref(),
            Some("recovery_exhausted")
        );
        // It names the operation and carries the last failure, not a placeholder.
        assert!(
            payload.message.contains("start"),
            "unexpected message: {}",
            payload.message
        );
        assert!(
            payload
                .error
                .as_deref()
                .is_some_and(|e| e.contains("2 attempts failed"))
        );
    }

    #[sqlx::test]
    async fn lock_contention_neither_spends_the_budget_nor_gates_the_next_tick(pool: PgPool) {
        // The idle reaper's stop holds the per-Task lock while it flushes the
        // workspace, and the reconciler ticks inside that window. Contention is
        // the holder driving this same generation, so the budget must survive
        // it: spending it published a `recovery_exhausted` error against a stop
        // that then completed, and refused to drive the operation again.
        seed(&pool, "t1", "open", "stopped").await;
        let (rec, _sink) = build_with(
            pool.clone(),
            pool,
            Arc::new(RecordingRuntime::new()),
            observer(&[], SandboxObservedStatus::Absent),
            Arc::new(RecordingNotify::default()),
            Arc::new(RecordingMemberRegistry::default()),
            Arc::new(SystemWallClock),
            ReconcilerConfig {
                grace: Duration::ZERO,
                attempt_cap: 2,
                ..Default::default()
            },
        );

        let now = Utc::now();
        for _ in 0..5 {
            assert!(
                rec.retry_gate("t1", Some(1), now),
                "contention must leave the next tick eligible"
            );
            let spent = rec.record_outcome(
                "t1",
                ReconcileAction::Stop,
                Err(LifecycleError::OperationInProgress {
                    kind: OperationKind::Stop,
                }),
                now,
            );
            assert!(
                spent.is_none(),
                "contention must not spend the budget or publish a give-up"
            );
        }
    }

    #[sqlx::test]
    async fn a_permanent_failure_reports_the_one_attempt_it_spent(pool: PgPool) {
        // The give-up event used to publish the cap rather than the count, so a
        // permanent failure on the first attempt read as five failed retries.
        seed(&pool, "t1", "open", "stopped").await;
        let (rec, _sink) = build_with(
            pool.clone(),
            pool,
            Arc::new(RecordingRuntime::new()),
            observer(&[], SandboxObservedStatus::Absent),
            Arc::new(RecordingNotify::default()),
            Arc::new(RecordingMemberRegistry::default()),
            Arc::new(SystemWallClock),
            ReconcilerConfig {
                grace: Duration::ZERO,
                attempt_cap: 5,
                ..Default::default()
            },
        );

        let now = Utc::now();
        assert!(rec.retry_gate("t1", Some(1), now));
        let spent = rec
            .record_outcome(
                "t1",
                ReconcileAction::Stop,
                Err(LifecycleError::TaskNotFound("t1".into())),
                now,
            )
            .expect("a permanent failure spends the budget");

        assert_eq!(spent.attempts, 1);
        assert!(
            !rec.retry_gate("t1", Some(1), now),
            "a permanent failure still blocks this generation"
        );
    }

    #[sqlx::test]
    async fn re_notifies_a_woken_ready_sandbox_bounded_by_backoff(pool: PgPool) {
        // Desired Running, observed Ready → converged, so no lifecycle operation.
        // A pending user message keeps the Task woken: the wake's gRPC notify may
        // have been lost while the sandbox was already Ready, so the reconciler
        // re-notifies. Backoff bounds it — a second immediate tick is spaced out.
        seed(&pool, "t1", "open", "running").await;
        seed_pending_user_message(&pool, "t1", Utc::now()).await;
        let runtime = Arc::new(RecordingRuntime::new());
        let notify = Arc::new(RecordingNotify::default());
        let (rec, _sink) = build_with(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(
                &[("t1", SandboxObservedStatus::Ready)],
                SandboxObservedStatus::Ready,
            ),
            notify.clone(),
            Arc::new(RecordingMemberRegistry::default()),
            Arc::new(SystemWallClock),
            ReconcilerConfig {
                grace: Duration::ZERO,
                ..Default::default()
            },
        );

        rec.reconcile_once().await;
        rec.reconcile_once().await;

        assert_eq!(
            runtime.calls(),
            (0, 0, 0),
            "converged — no lifecycle operation driven"
        );
        assert_eq!(
            *notify.count.lock().unwrap(),
            1,
            "notify fired once; the second immediate tick is backoff-suppressed",
        );
    }

    #[sqlx::test]
    async fn the_wake_reaches_the_shared_sandbox_member_holding_the_message(pool: PgPool) {
        // Two Tasks on one Sandbox: "t1" created it, "t2" joined it (a
        // `sameSandbox` fork). Only "t2" has an unclaimed message. The candidate
        // enumeration collapses the Sandbox to its oldest member, "t1", so a
        // notify aimed at the candidate would dispatch the wrong Task and leave
        // "t2" waiting forever while its message kept the wake set.
        seed(&pool, "t1", "open", "running").await;
        seed_bare_task(&pool, "t2", "open").await;
        sqlx::query("UPDATE tasks SET sandbox_id = 't1' WHERE task_id = 't2'")
            .execute(&pool)
            .await
            .unwrap();
        seed_pending_user_message(&pool, "t2", Utc::now()).await;

        let notify = Arc::new(RecordingNotify::default());
        let (rec, _sink) = build_with(
            pool.clone(),
            pool,
            Arc::new(RecordingRuntime::new()),
            observer(
                &[("t1", SandboxObservedStatus::Ready)],
                SandboxObservedStatus::Ready,
            ),
            notify.clone(),
            Arc::new(RecordingMemberRegistry::default()),
            Arc::new(SystemWallClock),
            ReconcilerConfig {
                grace: Duration::ZERO,
                ..Default::default()
            },
        );

        rec.reconcile_once().await;

        assert_eq!(
            notify.targets.lock().unwrap().as_slice(),
            ["t2"],
            "the wake goes to the member whose message is unclaimed, not the candidate"
        );
    }

    #[sqlx::test]
    async fn reissues_join_task_for_all_running_sandbox_members(pool: PgPool) {
        seed(&pool, "t1", "open", "running").await;
        seed_bare_task(&pool, "t2", "open").await;
        sqlx::query("UPDATE tasks SET sandbox_id = 't1' WHERE task_id = 't2'")
            .execute(&pool)
            .await
            .unwrap();
        let runtime = Arc::new(RecordingRuntime::new());
        let registry = Arc::new(RecordingMemberRegistry::default());
        let (rec, _sink) = build_with(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(
                &[("t1", SandboxObservedStatus::Ready)],
                SandboxObservedStatus::Ready,
            ),
            Arc::new(RecordingNotify::default()),
            registry.clone(),
            Arc::new(SystemWallClock),
            ReconcilerConfig {
                grace: Duration::ZERO,
                ..Default::default()
            },
        );

        rec.reconcile_once().await;

        let mut calls = registry.calls.lock().unwrap().clone();
        calls.sort_by(|a, b| a.task_id.cmp(&b.task_id));
        assert_eq!(
            calls,
            vec![
                SandboxMember {
                    task_id: "t1".to_string(),
                    sandbox_id: "t1".to_string(),
                    checkout_mode: moatless_types::task::CheckoutMode::Shared,
                },
                SandboxMember {
                    task_id: "t2".to_string(),
                    sandbox_id: "t1".to_string(),
                    checkout_mode: moatless_types::task::CheckoutMode::Shared,
                },
            ],
            "reconciler rebuilds in-pod members from DB membership after restart"
        );
    }

    #[sqlx::test]
    async fn stops_a_desired_stopped_task_observed_ready(pool: PgPool) {
        // Desired Stopped, but the fleet shows it live — surfaced as a candidate
        // by the fleet LIST, not by the DB candidate query.
        seed(&pool, "t1", "open", "stopped").await;
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, _sink) = build(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(
                &[("t1", SandboxObservedStatus::Ready)],
                SandboxObservedStatus::Ready,
            ),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        assert_eq!(runtime.calls().1, 1, "stop driven once");
    }

    #[sqlx::test]
    async fn removes_when_desired_removed_and_still_live(pool: PgPool) {
        seed(&pool, "t1", "closed", "removed").await;
        // Persist a Removed intent with a marker so it is a DB candidate.
        let store = PgLifecycleDao::new(pool.clone());
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: moatless_types::sandbox::SandboxDesiredState::Removed,
                    stop_reason: Some(SandboxStopReason::Cleanup),
                    retention: Some(ArchiveRetention::DeleteArchives),
                    marker: CorrelationMarker::new(
                        OperationKind::Remove,
                        OperationGeneration(1),
                        Utc::now(),
                    ),
                },
            )
            .await
            .unwrap();
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, _sink) = build(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(
                &[("t1", SandboxObservedStatus::Ready)],
                SandboxObservedStatus::Ready,
            ),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        assert_eq!(runtime.calls().2, 1, "remove driven once");
    }

    #[sqlx::test]
    async fn confirm_read_overrides_stale_coarse_and_skips(pool: PgPool) {
        // Coarse LIST omits the task (→ Absent → would start), but the
        // authoritative confirm read says Ready: converged, drive nothing.
        seed(&pool, "t1", "open", "running").await;
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, sink) = build(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(&[], SandboxObservedStatus::Ready),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        assert_eq!(runtime.calls(), (0, 0, 0), "no operation driven");
        assert!(sink.facts.lock().unwrap().is_empty());
    }

    #[sqlx::test]
    async fn grace_window_defers_a_freshly_changed_intent(pool: PgPool) {
        // Seed sets sandbox_desired_state_updated_at to NOW(); a long grace means
        // the reconciler leaves the caller's own in-flight op alone this tick.
        seed(&pool, "t1", "open", "running").await;
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, _sink) = build(
            pool.clone(),
            pool,
            runtime.clone(),
            observer(&[], SandboxObservedStatus::Absent),
            Duration::from_hours(1),
        );

        rec.reconcile_once().await;

        assert_eq!(runtime.calls(), (0, 0, 0), "grace deferred the operation");
    }

    #[sqlx::test]
    async fn replacement_generation_ready_acknowledges_edge_marker(pool: PgPool) {
        seed(&pool, "t1", "open", "running").await;
        let store = PgLifecycleDao::new(pool.clone());
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: CorrelationMarker::new(
                        OperationKind::Restart,
                        OperationGeneration(7),
                        Utc::now(),
                    ),
                },
            )
            .await
            .unwrap();
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, _sink) = build(
            pool.clone(),
            pool.clone(),
            runtime.clone(),
            observer_with_generation(
                "t1",
                SandboxObservedStatus::Ready,
                Some(OperationGeneration(7)),
            ),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        let marker: Option<String> =
            sqlx::query_scalar("SELECT operation_kind FROM sandboxes WHERE sandbox_id = 't1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(marker, None);
        assert_eq!(runtime.calls(), (0, 0, 0));
    }

    #[sqlx::test]
    async fn fresh_edge_marker_retains_suppression_and_is_not_replayed(pool: PgPool) {
        seed(&pool, "t1", "open", "running").await;
        let store = PgLifecycleDao::new(pool.clone());
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: CorrelationMarker::new(
                        OperationKind::Redeploy,
                        OperationGeneration(8),
                        Utc::now(),
                    ),
                },
            )
            .await
            .unwrap();
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, sink) = build(
            pool.clone(),
            pool.clone(),
            runtime.clone(),
            observer_with_generation("t1", SandboxObservedStatus::Starting, None),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        let marker: Option<String> =
            sqlx::query_scalar("SELECT operation_kind FROM sandboxes WHERE sandbox_id = 't1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(marker.as_deref(), Some("redeploy"));
        assert_eq!(runtime.calls(), (0, 0, 0));
        assert!(sink.facts.lock().unwrap().is_empty());
    }

    #[sqlx::test]
    async fn stale_edge_persists_error_before_clearing_marker(pool: PgPool) {
        seed(&pool, "t1", "open", "running").await;
        let store = PgLifecycleDao::new(pool.clone());
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: CorrelationMarker::new(
                        OperationKind::Restart,
                        OperationGeneration(9),
                        Utc::now() - chrono::Duration::minutes(20),
                    ),
                },
            )
            .await
            .unwrap();
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, sink) = build(
            pool.clone(),
            pool.clone(),
            runtime.clone(),
            observer_with_generation("t1", SandboxObservedStatus::Starting, None),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        let marker: Option<String> =
            sqlx::query_scalar("SELECT operation_kind FROM sandboxes WHERE sandbox_id = 't1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(marker, None);
        let is_error: bool = sqlx::query_scalar(
            "SELECT is_error FROM task_events WHERE event_id = 'sandbox:t1:9:error'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(is_error);
        assert_eq!(runtime.calls(), (0, 0, 0));
        assert_eq!(*sink.facts.lock().unwrap(), vec![EventFact::Error]);
    }

    #[sqlx::test]
    async fn edge_between_suppression_and_abandonment_bounds_is_retained(pool: PgPool) {
        seed(&pool, "t1", "open", "running").await;
        let store = PgLifecycleDao::new(pool.clone());
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: CorrelationMarker::new(
                        OperationKind::Restart,
                        OperationGeneration(10),
                        Utc::now() - chrono::Duration::minutes(10),
                    ),
                },
            )
            .await
            .unwrap();
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, sink) = build(
            pool.clone(),
            pool.clone(),
            runtime.clone(),
            observer_with_generation("t1", SandboxObservedStatus::Starting, None),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        let marker: Option<String> =
            sqlx::query_scalar("SELECT operation_kind FROM sandboxes WHERE sandbox_id = 't1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(marker.as_deref(), Some("restart"));
        assert!(sink.facts.lock().unwrap().is_empty());
        assert_eq!(runtime.calls(), (0, 0, 0));
    }

    #[sqlx::test]
    async fn old_edge_with_matching_replacement_still_starting_is_retained(pool: PgPool) {
        seed(&pool, "t1", "open", "running").await;
        let store = PgLifecycleDao::new(pool.clone());
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: CorrelationMarker::new(
                        OperationKind::Redeploy,
                        OperationGeneration(11),
                        Utc::now() - chrono::Duration::minutes(20),
                    ),
                },
            )
            .await
            .unwrap();
        let runtime = Arc::new(RecordingRuntime::new());
        let (rec, sink) = build(
            pool.clone(),
            pool.clone(),
            runtime.clone(),
            observer_with_generation(
                "t1",
                SandboxObservedStatus::Starting,
                Some(OperationGeneration(11)),
            ),
            Duration::ZERO,
        );

        rec.reconcile_once().await;

        let marker: Option<String> =
            sqlx::query_scalar("SELECT operation_kind FROM sandboxes WHERE sandbox_id = 't1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(marker.as_deref(), Some("redeploy"));
        assert!(sink.facts.lock().unwrap().is_empty());
        assert_eq!(runtime.calls(), (0, 0, 0));
    }
}
