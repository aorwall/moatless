//! Postgres [`LifecycleDao`] — durable intent, generation allocation, and the
//! commit-before-publication completion commit.
//!
//! The atomic intent write collapses desired state, stop reason, retention, and
//! the correlation marker into **one** statement, so readers never see partial
//! intent. `commit_fact_and_clear_marker` persists the terminal fact and clears
//! the marker in one transaction, so the deterministic event id is never live
//! without its persisted fact.

use std::collections::BTreeMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use moatless_core::db::DbPool;
use moatless_core::db_enum::{decode_optional, decode_required};
use moatless_types::access_control::Scope;
use moatless_types::sandbox::{SandboxDesiredState, SandboxStopReason, sandbox_intent_epoch};

use super::events::LifecycleEvent;
use super::orchestrator::{OutstandingCommand, StoreError, TaskIntent};
use super::reconciler::SandboxMember;
use super::token_generation::DriverChange;
use moatless_sandbox_runtime::operation::{
    CorrelationMarker, IntentEffect, OperationGeneration, OperationKind,
};
use moatless_types::sandbox::ArchiveRetention;

/// What a Sandbox's pod is built from, all of it off the `sandboxes` row.
///
/// [`Self::acting_user_id`] is the identity the pod's tokens are minted for;
/// `run_as_user_id` is the identity the pod acts as outward for git-host
/// credentials and delivered plugins.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct SandboxShape {
    pub workspace_id: String,
    #[sqlx(try_from = "String")]
    pub scope: Scope,
    pub owner_user_id: Option<String>,
    /// Holds the owner when nothing pinned another identity: the row is
    /// written `Task > Workspace > owner`. See [`Self::pinned_run_as`].
    pub run_as_user_id: Option<String>,
    /// Whether the Task pins a run-as, even one equal to the owner.
    pub run_as_pinned: bool,
    /// The person whose message started the current turn.
    pub driver_user_id: Option<String>,
    /// Whether the Sandbox's founding Task is public, which grants its driver
    /// write.
    pub public: bool,
    pub branch: Option<String>,
    pub headless: bool,
    pub agent_profile: Option<String>,
    pub use_tmux: bool,
    /// Extra node labels for the pod, already resolved Loop-over-Workspace at
    /// Sandbox creation so a pod rebuild keeps the placement it started with.
    pub node_selector: Option<sqlx::types::Json<BTreeMap<String, String>>>,
    /// Stop as soon as the agent run finishes, instead of holding the pod for
    /// the idle TTL. Set from the Loop that opened the Sandbox.
    pub stop_when_done: bool,
    /// The generation a task token for this Sandbox is minted at. See
    /// [`moatless_core::sandbox_jwt::check_generation`].
    pub token_generation: i64,
}

impl SandboxShape {
    /// The person the Sandbox acts for: its driver, else its owner.
    pub fn acting_user_id(&self) -> Option<&str> {
        self.driver_user_id
            .as_deref()
            .or(self.owner_user_id.as_deref())
    }

    /// The run-as identity pinned over the Sandbox's people, `None` when the
    /// row only holds its owner: the driver replaces the owner, not a pin.
    pub fn pinned_run_as(&self) -> Option<&str> {
        self.run_as_user_id
            .as_deref()
            .filter(|run_as| self.run_as_pinned || Some(*run_as) != self.owner_user_id.as_deref())
    }
}

/// One Sandbox the reconciler should look at, named by the Task that represents
/// it, plus when work was last queued for it.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ReconcileCandidate {
    pub task_id: String,
    pub pending_work_at: Option<DateTime<Utc>>,
}

/// The atomic intent write — desired state, reason, retention, and the
/// correlation marker, all in one statement/transaction so readers never
/// observe partial intent.
#[derive(Debug, Clone)]
pub struct IntentWrite {
    pub desired_state: SandboxDesiredState,
    pub stop_reason: Option<SandboxStopReason>,
    pub retention: Option<ArchiveRetention>,
    pub marker: CorrelationMarker,
}

/// Durable intent persistence + generation allocation.
#[async_trait]
pub trait LifecycleDao: Send + Sync {
    async fn load(&self, task_id: &str) -> Result<Option<TaskIntent>, StoreError>;

    /// DB-derived reconciler candidates for the reconciler to check.
    ///
    /// Tasks that desire Running, Sandboxes with an incomplete operation
    /// marker, and Sandboxes with queued work, unioned with the observed fleet
    /// to decide what diverged.
    async fn reconcile_candidates(&self) -> Result<Vec<ReconcileCandidate>, StoreError>;

    /// All open/error Task members of Sandboxes that durably desire Running.
    /// Used by the reconciler to rebuild in-pod member contexts after a pod
    /// restart from DB truth, not process memory.
    async fn running_sandbox_members(&self) -> Result<Vec<SandboxMember>, StoreError>;

    /// Whether another active Task still points at the same Sandbox.
    ///
    /// A member close/delete may stop that member's agent, but runtime
    /// resources belong to the Sandbox and stay while any open/error peer is
    /// still attached.
    async fn has_other_active_members(
        &self,
        sandbox_id: &str,
        task_id: &str,
    ) -> Result<bool, StoreError>;

    /// The Sandbox's pod shape and access-control pair. Absent when the Sandbox
    /// has no row, which is a Sandbox that was never created.
    async fn sandbox_shape(&self, sandbox_id: &str) -> Result<Option<SandboxShape>, StoreError>;

    /// Allocate the next durable operation generation for the Task.
    async fn allocate_generation(&self, task_id: &str) -> Result<OperationGeneration, StoreError>;

    /// Atomically persist intent + marker. Returns whether intent actually
    /// changed (desired state, stop reason, or retention).
    async fn persist_intent(
        &self,
        task_id: &str,
        write: &IntentWrite,
    ) -> Result<IntentEffect, StoreError>;

    /// Persist a full deterministic terminal fact and clear the marker.
    ///
    /// One transaction, for ordinary completion and a stale edge's abandonment
    /// fact; the caller publishes only after this returns `Ok`.
    async fn commit_fact_and_clear_marker(
        &self,
        sandbox_id: &str,
        task_id: &str,
        event: &LifecycleEvent,
    ) -> Result<(), StoreError>;

    /// Clear the marker synchronously — used for an `Unchanged` + `NoOp`
    /// against an already-converged target, where no completion fact will ever
    /// exist for the generation.
    async fn clear_marker(
        &self,
        task_id: &str,
        generation: OperationGeneration,
    ) -> Result<(), StoreError>;

    /// When each of `task_ids` entered `Stopped`, for the subset stopped now
    /// and still owning storage.
    ///
    /// Excludes a Task with no Sandbox row (never had storage) and one already
    /// reclaimed (would otherwise be re-offered every cycle).
    async fn stopped_with_storage(
        &self,
        task_ids: &[String],
    ) -> Result<Vec<(String, DateTime<Utc>)>, StoreError>;

    /// Record that the Task's storage was reclaimed, removing it from
    /// [`Self::stopped_with_storage`] until a start provisions a fresh volume.
    async fn mark_storage_reclaimed(&self, task_id: &str) -> Result<(), StoreError>;

    /// Record how many `moat cmd` commands the Sandbox is running, as the
    /// pod last reported. Absolute, so a missed report is corrected by the
    /// next one.
    ///
    /// Notifies the listing only when the count crosses zero: the sidebar
    /// renders "is anything running", and a 2→3 move wakes no client.
    async fn record_running_commands(&self, task_id: &str, running: i32) -> Result<(), StoreError>;

    /// Record whether the Sandbox's Codex reaches its model through the model
    /// gateway, as its pod last reported. A no-op when the Sandbox has no row.
    async fn record_codex_via_gateway(
        &self,
        sandbox_id: &str,
        via_gateway: bool,
    ) -> Result<(), StoreError>;

    /// Mirror the commands a pod says still owe its agent an outcome, on a
    /// row no failed workspace-snapshot restore touches, so a replacement
    /// can name them.
    ///
    /// Absolute, like [`Self::record_running_commands`]; `revision` is the
    /// pod registry lock's counter, and a lower-or-equal call is dropped.
    async fn record_unreported_commands(
        &self,
        task_id: &str,
        commands: &[OutstandingCommand],
        revision: i64,
    ) -> Result<(), StoreError>;

    /// Read the stored set and clear it in one statement, so a pod announces
    /// the loss exactly once however many times it restarts.
    async fn take_unreported_commands(
        &self,
        task_id: &str,
    ) -> Result<Vec<OutstandingCommand>, StoreError>;

    /// Advance the Sandbox's task-token generation and return the new value
    /// with the user its tokens are minted for, absent when the Sandbox has
    /// no row.
    async fn bump_token_generation(
        &self,
        sandbox_id: &str,
    ) -> Result<Option<(i64, Option<String>)>, StoreError>;

    /// Make `user_id` the Sandbox's driver and advance its generation in one
    /// statement. `None` when the Sandbox already acts for `user_id` or has
    /// no row.
    async fn switch_driver(
        &self,
        sandbox_id: &str,
        user_id: &str,
    ) -> Result<Option<DriverChange>, StoreError>;
}

/// Postgres-backed lifecycle store over the main pool.
pub struct PgLifecycleDao {
    pool: DbPool,
}

impl PgLifecycleDao {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
}

/// The statement that clears the correlation marker
/// (`kind/generation/started_at`). Shared by the standalone clear and the
/// completion transaction so the two cannot drift.
const CLEAR_MARKER_SQL: &str = r#"UPDATE sandboxes SET
       operation_kind = NULL,
       operation_generation = NULL,
       operation_started_at = NULL,
       updated_at = NOW()
   WHERE sandbox_id = $1 AND operation_generation = $2"#;

/// Clear the correlation marker on `executor`. Generic over the executor so the
/// completion transaction (`&mut tx`) and the standalone clear (`&pool`) issue
/// one and the same statement.
async fn clear_marker_on<'e, E>(
    executor: E,
    sandbox_id: &str,
    generation: OperationGeneration,
) -> Result<(), sqlx::Error>
where
    E: sqlx::Executor<'e, Database = sqlx::Postgres>,
{
    sqlx::query(CLEAR_MARKER_SQL)
        .bind(sandbox_id)
        .bind(generation.value())
        .execute(executor)
        .await
        .map(|_| ())
}

/// Reconstruct the persisted correlation marker from its three columns. Present
/// iff `sandbox_operation_kind` is set.
fn marker_from_columns(
    kind: Option<String>,
    generation: Option<i64>,
    started_at: Option<DateTime<Utc>>,
) -> Option<CorrelationMarker> {
    match (kind, generation, started_at) {
        (Some(kind), Some(generation), Some(started_at)) => OperationKind::from_db_str(&kind)
            .map(|kind| CorrelationMarker::new(kind, OperationGeneration(generation), started_at)),
        _ => None,
    }
}

#[async_trait]
impl LifecycleDao for PgLifecycleDao {
    async fn load(&self, task_id: &str) -> Result<Option<TaskIntent>, StoreError> {
        // Left join: a Task with no Sandbox row is not TaskNotFound — it reads as
        // converged Stopped (never requested) with an epoch intent timestamp and
        // an ownerless access-control pair, which only an admin can reach.
        let row: Option<(
            String,
            String,
            Option<String>,
            Option<String>,
            String,
            String,
            Option<bool>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
            Option<i64>,
            Option<DateTime<Utc>>,
            Option<DateTime<Utc>>,
        )> = sqlx::query_as(
            r#"SELECT
                t.status,
                t.visibility,
                s.scope,
                s.owner_user_id,
                COALESCE(t.sandbox_id, t.task_id) AS sandbox_id,
                COALESCE(t.workspace_id, 'ws_' || t.repository_id) AS workspace_id,
                s.headless,
                s.desired_state,
                s.stop_reason,
                s.archive_retention,
                s.operation_kind,
                s.operation_generation,
                s.operation_started_at,
                s.desired_state_updated_at
            FROM tasks t
            LEFT JOIN sandboxes s ON s.sandbox_id = COALESCE(t.sandbox_id, t.task_id)
            WHERE t.task_id = $1"#,
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::with_source("load lifecycle intent", e))?;

        row.map(
            |(
                status,
                visibility,
                scope,
                owner_user_id,
                sandbox_id,
                workspace_id,
                headless,
                desired_state,
                stop_reason,
                retention,
                op_kind,
                op_generation,
                op_started_at,
                desired_state_updated_at,
            )| {
                Ok::<_, sqlx::Error>(TaskIntent {
                    status: decode_required("status", status)?,
                    visibility: decode_required("visibility", visibility)?,
                    // No Sandbox row leaves the pair ownerless, which reads as
                    // reachable by an admin and nobody else.
                    scope: decode_optional("scope", scope)?.unwrap_or(Scope::User),
                    owner_user_id,
                    sandbox_id,
                    workspace_id,
                    headless: headless.unwrap_or(false),
                    desired_state: decode_optional("desired_state", desired_state)?
                        .unwrap_or(SandboxDesiredState::Stopped),
                    stop_reason: decode_optional("stop_reason", stop_reason)?,
                    retention: retention.as_deref().and_then(ArchiveRetention::from_db_str),
                    marker: marker_from_columns(op_kind, op_generation, op_started_at),
                    desired_state_updated_at: desired_state_updated_at
                        .unwrap_or_else(sandbox_intent_epoch),
                })
            },
        )
        .transpose()
        .map_err(|e| StoreError::with_source("load lifecycle intent", e))
    }

    async fn reconcile_candidates(&self) -> Result<Vec<ReconcileCandidate>, StoreError> {
        // Desired-running ∪ incomplete-marker ∪ pending-work sandboxes (the
        // last arm wakes one whose pod is down: the fact lives on the row).
        // One candidate per sandbox, not per task — lifecycle ops lock on
        // `sandbox_id`, so a second member would only contend the same lock.
        // The task with the work is found via `PendingWorkReader` separately.
        sqlx::query_as(
            r#"SELECT DISTINCT ON (s.sandbox_id) t.task_id, s.pending_work_at
               FROM sandboxes s
               JOIN tasks t ON t.sandbox_id = s.sandbox_id
               WHERE (s.desired_state = 'running' AND t.status IN ('open', 'error'))
                  OR (s.pending_work_at IS NOT NULL AND t.status IN ('open', 'error'))
                  OR s.operation_generation IS NOT NULL
               ORDER BY s.sandbox_id, t.created_at_db NULLS LAST, t.task_id"#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::with_source("load reconcile candidates", e))
    }

    async fn running_sandbox_members(&self) -> Result<Vec<SandboxMember>, StoreError> {
        let rows: Vec<SandboxMember> = sqlx::query_as(
            r#"SELECT
                   t.task_id,
                   s.sandbox_id,
                   COALESCE(t.checkout_mode, 'shared') AS checkout_mode,
                   t.agent_profile
               FROM sandboxes s
               JOIN tasks t ON t.sandbox_id = s.sandbox_id
               WHERE s.desired_state = 'running'
                 AND t.status IN ('open', 'error')
               ORDER BY s.sandbox_id, t.created_at_db NULLS LAST, t.task_id"#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::with_source("load running sandbox members", e))?;

        Ok(rows)
    }

    async fn has_other_active_members(
        &self,
        sandbox_id: &str,
        task_id: &str,
    ) -> Result<bool, StoreError> {
        sqlx::query_scalar(
            r#"SELECT EXISTS (
                   SELECT 1
                   FROM tasks
                   WHERE sandbox_id = $1
                     AND task_id <> $2
                     AND status IN ('open', 'error')
               )"#,
        )
        .bind(sandbox_id)
        .bind(task_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|e| StoreError::with_source("load active sandbox peers", e))
    }

    async fn sandbox_shape(&self, sandbox_id: &str) -> Result<Option<SandboxShape>, StoreError> {
        sqlx::query_as(
            r#"SELECT s.workspace_id, s.scope, s.owner_user_id, s.run_as_user_id,
                      (t.run_as_user_id IS NOT NULL) AS run_as_pinned,
                      s.driver_user_id,
                      COALESCE(t.visibility = 'public', false) AS public,
                      s.branch, s.headless, s.agent_profile, s.use_tmux,
                      s.node_selector, s.stop_when_done, s.token_generation
               FROM sandboxes s
               LEFT JOIN tasks t ON t.task_id = s.sandbox_id
               WHERE s.sandbox_id = $1"#,
        )
        .bind(sandbox_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::with_source("load sandbox shape", e))
    }

    async fn allocate_generation(
        &self,
        sandbox_id: &str,
    ) -> Result<OperationGeneration, StoreError> {
        // Task creation writes the Sandbox row, so allocation only increments.
        // Serialized per Sandbox by the lifecycle lock, so no concurrent
        // allocation races.
        let seq: Option<i64> = sqlx::query_scalar(
            r#"UPDATE sandboxes
                  SET operation_seq = operation_seq + 1,
                      updated_at = NOW()
                WHERE sandbox_id = $1
            RETURNING operation_seq"#,
        )
        .bind(sandbox_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::with_source("allocate operation generation", e))?;

        seq.map(OperationGeneration).ok_or_else(|| {
            StoreError::new(format!(
                "sandbox {sandbox_id} has no row allocating generation"
            ))
        })
    }

    async fn persist_intent(
        &self,
        sandbox_id: &str,
        write: &IntentWrite,
    ) -> Result<IntentEffect, StoreError> {
        let task_id = sandbox_id;
        // One atomic statement against the row task creation wrote. `prev`
        // captures the pre-image under a row lock, so `changed` reflects
        // only the desired-state/stop-reason/retention delta; the marker
        // is always overwritten. Now in a transaction, since the listing
        // record below must commit with it, or the sidebar stops updating.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StoreError::with_source("begin intent transaction", e))?;
        let changed: Option<bool> = sqlx::query_scalar(
            r#"WITH prev AS (
                   SELECT desired_state, stop_reason, archive_retention
                   FROM sandboxes
                   WHERE sandbox_id = $1
                   FOR UPDATE
               ),
               write AS (
                   UPDATE sandboxes s SET
                       desired_state = $2,
                       stop_reason = $3,
                       archive_retention = $4,
                       operation_kind = $5,
                       operation_generation = $6,
                       operation_started_at = $7,
                       started_at = CASE WHEN $8 THEN NOW() ELSE s.started_at END,
                       desired_state_updated_at = NOW(),
                       updated_at = NOW(),
                       -- A start provisions a fresh volume, so the reclaim mark
                       -- is stale the moment intent moves toward Running.
                       storage_reclaimed_at =
                           CASE WHEN $8 THEN NULL ELSE s.storage_reclaimed_at END,
                       -- A moved desired state means a pod that is going away or
                       -- coming up new, and the command registry lives in that
                       -- pod's memory. Whatever it was running is gone with it.
                       running_commands =
                           CASE WHEN (SELECT desired_state FROM prev) IS DISTINCT FROM $2
                                THEN 0 ELSE s.running_commands END
                   WHERE s.sandbox_id = $1
                   RETURNING 1
               )
               SELECT
                   (SELECT desired_state FROM prev) IS DISTINCT FROM $2
                   OR (SELECT stop_reason FROM prev) IS DISTINCT FROM $3
                   OR (SELECT archive_retention FROM prev) IS DISTINCT FROM $4
               FROM write"#,
        )
        .bind(task_id)
        .bind(write.desired_state.as_str())
        .bind(write.stop_reason.map(SandboxStopReason::as_str))
        .bind(write.retention.map(ArchiveRetention::as_str))
        .bind(write.marker.kind.as_str())
        .bind(write.marker.generation.value())
        .bind(write.marker.started_at)
        // Lifecycle owns the start timestamp: bump it whenever intent moves
        // toward Running (start/restart/redeploy), regardless of caller path.
        .bind(write.desired_state == SandboxDesiredState::Running)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| StoreError::with_source("persist lifecycle intent", e))?;

        // Only when intent actually moved: the marker is rewritten on every
        // operation, and `changed` is the axis that reaches the session status.
        // Last, after the `sandboxes` row this statement holds `FOR UPDATE`, per
        // the lock order in `moatless_core::change_feed`.
        if changed == Some(true) {
            // The conversation shows the same axis as its session status, so it
            // is told too — ahead of the listing's allocator, per the lock order
            // in `moatless_core::change_feed`.
            moatless_core::change_feed::conversation::record_session_change(&mut tx, task_id)
                .await
                .map_err(|e| StoreError::with_source("record conversation change for intent", e))?;
            moatless_core::change_feed::listing::record_change(&mut tx, task_id)
                .await
                .map_err(|e| StoreError::with_source("record listing change for intent", e))?;
        }
        tx.commit()
            .await
            .map_err(|e| StoreError::with_source("commit intent transaction", e))?;

        match changed {
            Some(true) => Ok(IntentEffect::Changed),
            Some(false) => Ok(IntentEffect::Unchanged),
            None => Err(StoreError::new(format!(
                "sandbox {sandbox_id} has no row persisting intent"
            ))),
        }
    }

    async fn commit_fact_and_clear_marker(
        &self,
        sandbox_id: &str,
        task_id: &str,
        event: &LifecycleEvent,
    ) -> Result<(), StoreError> {
        let payload = serde_json::to_value(event.payload())
            .map_err(|e| StoreError::with_source("serialize completion payload", e))?;

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StoreError::with_source("begin completion transaction", e))?;

        // Persist the deterministic completion fact. ON CONFLICT keeps the commit
        // idempotent across at-least-once retries of the same generation.
        sqlx::query(
            r#"INSERT INTO task_events (
                   event_id, task_id, type, sub_type, is_error,
                   agent_profile, turn_number, payload, timestamp
               ) VALUES ($1, $2, 'sandbox', $3, $4, NULL, NULL, $5, $6)
               ON CONFLICT (event_id) DO NOTHING"#,
        )
        .bind(&event.event_id)
        .bind(task_id)
        .bind(event.sub_type())
        .bind(event.is_error())
        .bind(&payload)
        .bind(Utc::now().to_rfc3339())
        .execute(&mut *tx)
        .await
        .map_err(|e| StoreError::with_source("persist completion fact", e))?;

        // Clear the marker in the same transaction: the fact is durable and the
        // operation is acknowledged together.
        clear_marker_on(&mut *tx, sandbox_id, event.generation)
            .await
            .map_err(|e| StoreError::with_source("clear marker on completion", e))?;

        tx.commit()
            .await
            .map_err(|e| StoreError::with_source("commit completion transaction", e))
    }

    async fn clear_marker(
        &self,
        sandbox_id: &str,
        generation: OperationGeneration,
    ) -> Result<(), StoreError> {
        clear_marker_on(&self.pool, sandbox_id, generation)
            .await
            .map_err(|e| StoreError::with_source("clear marker", e))?;
        Ok(())
    }

    async fn stopped_with_storage(
        &self,
        task_ids: &[String],
    ) -> Result<Vec<(String, DateTime<Utc>)>, StoreError> {
        if task_ids.is_empty() {
            return Ok(Vec::new());
        }
        // Inner join on `sandboxes`, so a Task that never had a Sandbox row is
        // absent rather than reading as converged Stopped the way `load` does:
        // here the question is which storage exists, not what is desired.
        // Backed by `sandboxes_reclaimable_idx`.
        sqlx::query_as(
            r#"SELECT t.task_id, s.desired_state_updated_at
               FROM tasks t
               JOIN sandboxes s ON s.sandbox_id = t.sandbox_id
               WHERE t.task_id = ANY($1)
                 AND desired_state = 'stopped'
                 AND storage_reclaimed_at IS NULL"#,
        )
        .bind(task_ids)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::with_source("load stopped sandboxes with storage", e))
    }

    async fn mark_storage_reclaimed(&self, sandbox_id: &str) -> Result<(), StoreError> {
        sqlx::query(
            r#"UPDATE sandboxes SET storage_reclaimed_at = NOW(), updated_at = NOW()
               WHERE sandbox_id = $1"#,
        )
        .bind(sandbox_id)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| StoreError::with_source("mark storage reclaimed", e))
    }

    async fn record_codex_via_gateway(
        &self,
        sandbox_id: &str,
        via_gateway: bool,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE sandboxes SET codex_via_gateway = $2
             WHERE sandbox_id = $1 AND codex_via_gateway <> $2",
        )
        .bind(sandbox_id)
        .bind(via_gateway)
        .execute(&self.pool)
        .await
        .map_err(|e| StoreError::with_source("record codex routing", e))?;
        Ok(())
    }

    async fn record_running_commands(
        &self,
        sandbox_id: &str,
        running: i32,
    ) -> Result<(), StoreError> {
        // In a transaction for the reason `persist_intent` is: the listing
        // record has to commit with the row it describes, or the sidebar holds
        // a count nothing will tell it to re-read.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StoreError::with_source("begin running-commands transaction", e))?;

        // Two statements, deliberately: a `FOR UPDATE` CTE beside the `UPDATE`
        // can run after it, skips the row that statement already modified, and
        // reads NULL, so every report would count as crossing zero.
        let previous: Option<i32> = sqlx::query_scalar(
            "SELECT running_commands FROM sandboxes WHERE sandbox_id = $1 FOR UPDATE",
        )
        .bind(sandbox_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| StoreError::with_source("lock running commands", e))?;
        let Some(previous) = previous else {
            return Ok(());
        };

        sqlx::query(
            "UPDATE sandboxes SET running_commands = $2, updated_at = NOW() \
             WHERE sandbox_id = $1",
        )
        .bind(sandbox_id)
        .bind(running)
        .execute(&mut *tx)
        .await
        .map_err(|e| StoreError::with_source("persist running commands", e))?;

        // The busy axis, not the count: a 2→3 move wakes no client.
        if (previous > 0) != (running > 0) {
            // Ahead of the listing's allocator, per the lock order in
            // `moatless_core::change_feed`.
            moatless_core::change_feed::conversation::record_session_change(&mut tx, sandbox_id)
                .await
                .map_err(|e| {
                    StoreError::with_source("record conversation change for commands", e)
                })?;
            moatless_core::change_feed::listing::record_change(&mut tx, sandbox_id)
                .await
                .map_err(|e| StoreError::with_source("record listing change for commands", e))?;
        }

        tx.commit()
            .await
            .map_err(|e| StoreError::with_source("commit running-commands transaction", e))
    }

    async fn record_unreported_commands(
        &self,
        task_id: &str,
        commands: &[OutstandingCommand],
        revision: i64,
    ) -> Result<(), StoreError> {
        let payload = serde_json::to_value(commands)
            .map_err(|e| StoreError::with_source("encode the unreported commands", e))?;

        // The revision predicate makes a late write harmless: the pod
        // writes off its registry lock, so an older, larger set can land
        // after a newer one and resurrect a command already told to the
        // agent. Strict `<`: a resent revision is a no-op, not a rewrite.
        sqlx::query(
            r#"INSERT INTO sandbox_unreported_commands (task_id, commands, revision)
               VALUES ($1, $2, $3)
               ON CONFLICT (task_id) DO UPDATE SET
                   commands = EXCLUDED.commands,
                   revision = EXCLUDED.revision,
                   updated_at = NOW()
               WHERE sandbox_unreported_commands.revision < EXCLUDED.revision"#,
        )
        .bind(task_id)
        .bind(&payload)
        .bind(revision)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(|e| StoreError::with_source("persist the unreported commands", e))
    }

    async fn take_unreported_commands(
        &self,
        task_id: &str,
    ) -> Result<Vec<OutstandingCommand>, StoreError> {
        // Read and clear in one statement, so two racing starts can't both
        // announce the same loss. `revision` is deliberately left as-is: a
        // write from the dying pod can still land after this, and resetting
        // it would resurrect what the agent's already been shown. The
        // replacement seeds its own counter from the clock, outrunning it.
        let stored: Option<serde_json::Value> = sqlx::query_scalar(
            r#"WITH prev AS (
                   SELECT task_id, commands
                   FROM sandbox_unreported_commands
                   WHERE task_id = $1
                   FOR UPDATE
               )
               UPDATE sandbox_unreported_commands d SET
                   commands = '[]'::jsonb,
                   updated_at = NOW()
               FROM prev
               WHERE d.task_id = prev.task_id
               RETURNING prev.commands"#,
        )
        .bind(task_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::with_source("take the unreported commands", e))?;

        let Some(stored) = stored else {
            return Ok(Vec::new());
        };
        serde_json::from_value(stored)
            .map_err(|e| StoreError::with_source("decode the unreported commands", e))
    }

    async fn bump_token_generation(
        &self,
        sandbox_id: &str,
    ) -> Result<Option<(i64, Option<String>)>, StoreError> {
        sqlx::query_as(
            r#"UPDATE sandboxes
                  SET token_generation = token_generation + 1,
                      updated_at = NOW()
                WHERE sandbox_id = $1
            RETURNING token_generation, COALESCE(driver_user_id, owner_user_id)"#,
        )
        .bind(sandbox_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::with_source("bump token generation", e))
    }

    async fn switch_driver(
        &self,
        sandbox_id: &str,
        user_id: &str,
    ) -> Result<Option<DriverChange>, StoreError> {
        // `FOR UPDATE` serializes two switches: the later one re-reads the
        // row the earlier committed, so a repeat of the same driver is no
        // change and bumps nothing.
        sqlx::query_as(
            r#"WITH previous AS (
                   SELECT sandbox_id,
                          COALESCE(driver_user_id, owner_user_id) AS previous_user_id
                     FROM sandboxes
                    WHERE sandbox_id = $1
                      FOR UPDATE
               )
               UPDATE sandboxes s
                  SET driver_user_id = $2,
                      token_generation = s.token_generation + 1,
                      updated_at = NOW()
                 FROM previous p
                WHERE s.sandbox_id = p.sandbox_id
                  AND p.previous_user_id IS DISTINCT FROM $2
            RETURNING s.token_generation AS generation, p.previous_user_id"#,
        )
        .bind(sandbox_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StoreError::with_source("switch the sandbox driver", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts() -> DateTime<Utc> {
        Utc.timestamp_opt(1_700_000_000, 0)
            .single()
            .expect("valid ts")
    }

    #[test]
    fn marker_reconstructs_only_when_all_columns_present() {
        let full = marker_from_columns(Some("stop".to_string()), Some(7), Some(ts()));
        let marker = full.expect("full marker");
        assert_eq!(marker.kind, OperationKind::Stop);
        assert_eq!(marker.generation, OperationGeneration(7));

        // A partial marker (any column NULL) is no marker.
        assert!(marker_from_columns(None, Some(7), Some(ts())).is_none());
        assert!(marker_from_columns(Some("stop".to_string()), None, Some(ts())).is_none());
        assert!(marker_from_columns(Some("stop".to_string()), Some(7), None).is_none());
        // An unknown kind slug is not a valid marker.
        assert!(marker_from_columns(Some("bogus".to_string()), Some(7), Some(ts())).is_none());
    }

    #[test]
    fn a_pin_counts_even_when_it_names_the_owner() {
        let shape = |run_as: &str, pinned| SandboxShape {
            workspace_id: "ws".to_string(),
            scope: Scope::Global,
            owner_user_id: Some("owner".to_string()),
            run_as_user_id: Some(run_as.to_string()),
            run_as_pinned: pinned,
            driver_user_id: Some("visitor".to_string()),
            public: true,
            branch: None,
            headless: false,
            agent_profile: None,
            use_tmux: false,
            node_selector: None,
            stop_when_done: false,
            token_generation: 1,
        };
        assert_eq!(shape("owner", false).pinned_run_as(), None);
        assert_eq!(shape("owner", true).pinned_run_as(), Some("owner"));
        assert_eq!(shape("bot", false).pinned_run_as(), Some("bot"));
    }
}

/// Postgres-backed store tests, gated on `db-tests`.
///
/// Run in isolation via `#[sqlx::test]` against an isolated DB with
/// `backend/migrations` applied.
#[cfg(all(test, feature = "db-tests"))]
mod db_tests {
    use super::super::test_support::{seed_bare_task, seed_task as seed};
    use super::*;
    use moatless_sandbox_runtime::observation_events::EventFact;
    use moatless_types::sandbox::SandboxStatus;
    use sqlx::PgPool;

    fn marker(kind: OperationKind, generation: i64) -> CorrelationMarker {
        CorrelationMarker::new(kind, OperationGeneration(generation), Utc::now())
    }

    #[sqlx::test]
    async fn allocate_generation_is_monotonic(pool: PgPool) {
        seed(&pool, "t1").await;
        let store = PgLifecycleDao::new(pool);
        assert_eq!(
            store.allocate_generation("t1").await.unwrap(),
            OperationGeneration(1)
        );
        assert_eq!(
            store.allocate_generation("t1").await.unwrap(),
            OperationGeneration(2)
        );
        assert_eq!(
            store.allocate_generation("t1").await.unwrap(),
            OperationGeneration(3)
        );
    }

    #[sqlx::test]
    async fn persist_intent_change_then_unchanged_and_load_round_trips(pool: PgPool) {
        seed(&pool, "t1").await;
        let store = PgLifecycleDao::new(pool);
        let write = IntentWrite {
            desired_state: SandboxDesiredState::Stopped,
            stop_reason: Some(SandboxStopReason::User),
            retention: None,
            marker: marker(OperationKind::Stop, 1),
        };
        assert_eq!(
            store.persist_intent("t1", &write).await.unwrap(),
            IntentEffect::Changed
        );
        // Same intent, fresh marker → the intent axis is Unchanged even though the
        // marker moved (Changed covers only desired-state / reason / retention).
        let write2 = IntentWrite {
            marker: marker(OperationKind::Stop, 2),
            ..write.clone()
        };
        assert_eq!(
            store.persist_intent("t1", &write2).await.unwrap(),
            IntentEffect::Unchanged
        );

        let loaded = store.load("t1").await.unwrap().expect("intent");
        assert_eq!(loaded.desired_state, SandboxDesiredState::Stopped);
        assert_eq!(loaded.stop_reason, Some(SandboxStopReason::User));
        let m = loaded.marker.expect("marker");
        assert_eq!(m.kind, OperationKind::Stop);
        assert_eq!(m.generation, OperationGeneration(2));
    }

    #[sqlx::test]
    async fn remove_persists_retention_and_fact_commit_clears_marker(pool: PgPool) {
        seed(&pool, "t1").await;
        let store = PgLifecycleDao::new(pool.clone());
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Removed,
                    stop_reason: Some(SandboxStopReason::Cleanup),
                    retention: Some(ArchiveRetention::RetainArchives),
                    marker: marker(OperationKind::Remove, 5),
                },
            )
            .await
            .unwrap();

        let event = LifecycleEvent {
            event_id: "sandbox:t1:5:removed".to_string(),
            generation: OperationGeneration(5),
            fact: EventFact::Removed,
            status: SandboxStatus::Removed,
            stop_reason: None,
            failure: None,
        };
        store
            .commit_fact_and_clear_marker("t1", "t1", &event)
            .await
            .unwrap();
        // At-least-once retry of the same generation must not duplicate the row.
        store
            .commit_fact_and_clear_marker("t1", "t1", &event)
            .await
            .unwrap();

        let after = store.load("t1").await.unwrap().unwrap();
        assert!(after.marker.is_none(), "completion clears the marker");
        // Desired state + retention survive the marker clear.
        assert_eq!(after.desired_state, SandboxDesiredState::Removed);
        assert_eq!(after.retention, Some(ArchiveRetention::RetainArchives));

        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM task_events WHERE event_id = $1")
            .bind("sandbox:t1:5:removed")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 1, "deterministic id persists exactly once");
    }

    #[sqlx::test]
    async fn clear_marker_only_clears_the_matching_generation(pool: PgPool) {
        seed(&pool, "t1").await;
        let store = PgLifecycleDao::new(pool);
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: marker(OperationKind::Start, 1),
                },
            )
            .await
            .unwrap();
        assert!(store.load("t1").await.unwrap().unwrap().marker.is_some());
        store
            .clear_marker("t1", OperationGeneration(2))
            .await
            .unwrap();
        assert!(
            store.load("t1").await.unwrap().unwrap().marker.is_some(),
            "late acknowledgement must not clear a newer marker"
        );
        store
            .clear_marker("t1", OperationGeneration(1))
            .await
            .unwrap();
        let after = store.load("t1").await.unwrap().unwrap();
        assert!(after.marker.is_none());
        // Desired state is untouched by a marker clear.
        assert_eq!(after.desired_state, SandboxDesiredState::Running);
    }

    #[sqlx::test]
    async fn late_fact_commit_does_not_clear_a_newer_generation(pool: PgPool) {
        seed(&pool, "t1").await;
        let store = PgLifecycleDao::new(pool);
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: marker(OperationKind::Restart, 2),
                },
            )
            .await
            .unwrap();

        let old_event = LifecycleEvent {
            event_id: "sandbox:t1:1:error".to_string(),
            generation: OperationGeneration(1),
            fact: EventFact::Error,
            status: SandboxStatus::Error,
            stop_reason: None,
            failure: None,
        };
        store
            .commit_fact_and_clear_marker("t1", "t1", &old_event)
            .await
            .unwrap();

        let after = store.load("t1").await.unwrap().unwrap();
        assert_eq!(
            after.marker.map(|marker| marker.generation),
            Some(OperationGeneration(2))
        );
    }

    #[sqlx::test]
    async fn reconcile_candidates_covers_desired_running_participants_and_incomplete_markers(
        pool: PgPool,
    ) {
        use super::super::test_support::seed_task_with;
        // Desired Running + Open → candidate.
        seed_task_with(&pool, "run-open", "open", "running").await;
        // Desired Running + Error → candidate (Error participates).
        seed_task_with(&pool, "run-error", "error", "running").await;
        // Desired Running + Closed → not a candidate (does not participate).
        seed_task_with(&pool, "run-closed", "closed", "running").await;
        // Desired Stopped + Open → not a candidate on the desired-Running branch…
        seed_task_with(&pool, "stopped-open", "open", "stopped").await;
        // …until it carries an incomplete operation marker (a crashed stop/remove).
        let store = PgLifecycleDao::new(pool.clone());
        store
            .persist_intent(
                "stopped-open",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Stopped,
                    stop_reason: Some(SandboxStopReason::User),
                    retention: None,
                    marker: marker(OperationKind::Stop, 1),
                },
            )
            .await
            .unwrap();

        let mut got: Vec<String> = store
            .reconcile_candidates()
            .await
            .unwrap()
            .into_iter()
            .map(|c| c.task_id)
            .collect();
        got.sort();
        assert_eq!(got, vec!["run-error", "run-open", "stopped-open"]);
    }

    #[sqlx::test]
    async fn toward_running_persist_stamps_started_at_and_non_running_preserves_it(pool: PgPool) {
        seed(&pool, "t1").await;
        let store = PgLifecycleDao::new(pool.clone());
        let started_at = |pool: PgPool| async move {
            sqlx::query_scalar::<_, Option<chrono::DateTime<Utc>>>(
                "SELECT started_at FROM sandboxes WHERE sandbox_id = 't1'",
            )
            .fetch_one(&pool)
            .await
            .unwrap()
        };
        assert!(
            started_at(pool.clone()).await.is_none(),
            "seed leaves it unset"
        );

        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: marker(OperationKind::Start, 1),
                },
            )
            .await
            .unwrap();
        let stamped = started_at(pool.clone())
            .await
            .expect("toward-running stamps started_at");

        // A later stop keeps the original start timestamp — only toward-running writes it.
        store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Stopped,
                    stop_reason: Some(SandboxStopReason::User),
                    retention: None,
                    marker: marker(OperationKind::Stop, 2),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            started_at(pool).await,
            Some(stamped),
            "non-running persist preserves started_at"
        );
    }

    // ---- Persistence matrix ----

    async fn sandbox_row_count(pool: &PgPool, task_id: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM sandboxes WHERE sandbox_id = $1")
            .bind(task_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[sqlx::test]
    async fn lifecycle_writes_intent_onto_the_row_task_creation_made(pool: PgPool) {
        // Task creation writes the row; the first lifecycle operation states
        // intent on it rather than creating a second one.
        seed_bare_task(&pool, "t1", "open").await;
        assert_eq!(
            sandbox_row_count(&pool, "t1").await,
            1,
            "task creation wrote the row"
        );

        let store = PgLifecycleDao::new(pool.clone());
        let effect = store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: marker(OperationKind::Start, 1),
                },
            )
            .await
            .unwrap();
        assert_eq!(
            effect,
            IntentEffect::Changed,
            "stopped -> running is a change"
        );
        assert_eq!(sandbox_row_count(&pool, "t1").await, 1, "still one row");
    }

    #[sqlx::test]
    async fn a_sandbox_with_no_row_refuses_allocation_and_intent(pool: PgPool) {
        // No row means no Sandbox. Lifecycle names it rather than creating one
        // whose shape it would have to guess.
        seed_bare_task(&pool, "t1", "open").await;
        sqlx::query("UPDATE tasks SET sandbox_id = NULL WHERE task_id = 't1'")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM sandboxes WHERE sandbox_id = 't1'")
            .execute(&pool)
            .await
            .unwrap();
        let store = PgLifecycleDao::new(pool.clone());

        let allocate = store.allocate_generation("t1").await.unwrap_err();
        assert!(
            allocate.context.contains("t1"),
            "error names the sandbox: {allocate}"
        );

        let persist = store
            .persist_intent(
                "t1",
                &IntentWrite {
                    desired_state: SandboxDesiredState::Running,
                    stop_reason: None,
                    retention: None,
                    marker: marker(OperationKind::Start, 1),
                },
            )
            .await
            .unwrap_err();
        assert!(
            persist.context.contains("t1"),
            "error names the sandbox: {persist}"
        );
    }

    #[sqlx::test]
    async fn load_reads_access_control_off_the_sandbox_and_visibility_off_the_task(pool: PgPool) {
        // One read answers who may reach the Sandbox. The pair is the Sandbox's
        // own; `visibility` is the Task's, because a public Task grants read.
        seed_bare_task(&pool, "t1", "open").await;
        sqlx::query(
            "UPDATE sandboxes SET scope = 'global', owner_user_id = NULL WHERE sandbox_id = 't1'",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("UPDATE tasks SET visibility = 'public' WHERE task_id = 't1'")
            .execute(&pool)
            .await
            .unwrap();
        let store = PgLifecycleDao::new(pool.clone());

        let intent = store.load("t1").await.unwrap().expect("task exists");
        assert_eq!(intent.scope, Scope::Global);
        assert_eq!(intent.owner_user_id, None);
        assert_eq!(intent.visibility, moatless_types::task::Visibility::Public);

        // With the Sandbox row gone the pair is ownerless, which reaches nobody
        // but an admin.
        sqlx::query("DELETE FROM sandboxes WHERE sandbox_id = 't1'")
            .execute(&pool)
            .await
            .unwrap();
        let intent = store.load("t1").await.unwrap().expect("task exists");
        assert_eq!(intent.scope, Scope::User);
        assert_eq!(intent.owner_user_id, None);
    }

    #[sqlx::test]
    async fn a_never_operated_sandbox_is_converged_stopped_and_no_candidate(pool: PgPool) {
        // The row task creation writes reads as never-requested: converged
        // Stopped at the intent epoch (so any Message supersedes it), and the
        // Task is not a reconciler candidate.
        seed_bare_task(&pool, "t1", "open").await;
        let store = PgLifecycleDao::new(pool.clone());

        let intent = store.load("t1").await.unwrap().expect("task exists");
        assert_eq!(intent.desired_state, SandboxDesiredState::Stopped);
        assert!(intent.marker.is_none());
        assert_eq!(intent.stop_reason, None);
        assert_eq!(
            intent.desired_state_updated_at,
            sandbox_intent_epoch(),
            "no intent timestamp"
        );

        assert!(
            store.reconcile_candidates().await.unwrap().is_empty(),
            "a never-operated Sandbox is converged, not a candidate",
        );
    }

    #[sqlx::test]
    async fn generation_allocation_leaves_intent_at_epoch(pool: PgPool) {
        seed_bare_task(&pool, "t1", "open").await;
        let store = PgLifecycleDao::new(pool.clone());

        assert_eq!(
            store.allocate_generation("t1").await.unwrap(),
            OperationGeneration(1)
        );

        let intent = store.load("t1").await.unwrap().expect("task exists");
        assert_eq!(intent.desired_state, SandboxDesiredState::Stopped);
        assert_eq!(intent.desired_state_updated_at, sandbox_intent_epoch());
    }

    #[sqlx::test]
    async fn sandbox_row_survives_task_deletion(pool: PgPool) {
        seed(&pool, "t1").await;
        assert_eq!(sandbox_row_count(&pool, "t1").await, 1);
        sqlx::query("DELETE FROM tasks WHERE task_id = 't1'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(
            sandbox_row_count(&pool, "t1").await,
            1,
            "a Sandbox row is independent of any one member Task",
        );
    }

    /// A sidebar shows whether a session is running, so intent moving is a
    /// listing change and intent *not* moving is not.
    ///
    /// Recording on the call rather than the effect would advance the
    /// deployment's position counter for bookkeeping churn no sidebar shows.
    #[sqlx::test]
    async fn intent_records_a_listing_change_only_when_it_actually_moves(pool: PgPool) {
        seed(&pool, "t1").await;
        let store = PgLifecycleDao::new(pool.clone());
        // `seed` leaves the Sandbox running, so stopping it is the move.
        let write = IntentWrite {
            desired_state: SandboxDesiredState::Stopped,
            stop_reason: Some(SandboxStopReason::User),
            retention: None,
            marker: marker(OperationKind::Stop, 1),
        };
        assert_eq!(
            store.persist_intent("t1", &write).await.unwrap(),
            IntentEffect::Changed
        );

        let changes = moatless_core::change_feed::listing::changes_after(&pool, 0, 10)
            .await
            .unwrap();
        assert_eq!(
            changes.len(),
            1,
            "the session moved and nothing recorded it"
        );
        assert_eq!(changes[0].task_id, "t1");
        let after_move = moatless_core::change_feed::listing::current_position(&pool)
            .await
            .unwrap();

        // Same intent, a fresh marker: the correlation churn every operation
        // writes, and nothing a sidebar carries.
        let again = IntentWrite {
            marker: marker(OperationKind::Stop, 2),
            ..write.clone()
        };
        assert_eq!(
            store.persist_intent("t1", &again).await.unwrap(),
            IntentEffect::Unchanged
        );
        assert_eq!(
            moatless_core::change_feed::listing::current_position(&pool)
                .await
                .unwrap(),
            after_move,
            "rewriting the marker advanced the deployment's position counter"
        );
    }

    /// `record_change`'s "call this last" rule, for the one mutation site an
    /// integration test cannot reach since `PgLifecycleDao` is `pub(crate)`.
    ///
    /// Holds the `sandboxes` row `FOR UPDATE`, then takes the counter on a
    /// third connection; reversing that order would deadlock with every
    /// conversation mutation.
    #[sqlx::test]
    async fn an_intent_write_blocked_on_the_sandbox_row_is_not_holding_the_counter(pool: PgPool) {
        use moatless_core::change_feed::listing::test_support::{
            Probe, probe_counter, waits_for_a_lock,
        };

        seed(&pool, "t1").await;

        let mut holder = pool.begin().await.unwrap();
        sqlx::query("SELECT sandbox_id FROM sandboxes WHERE sandbox_id = 't1' FOR UPDATE")
            .execute(&mut *holder)
            .await
            .unwrap();

        let store = PgLifecycleDao::new(pool.clone());
        let write = tokio::spawn(async move {
            store
                .persist_intent(
                    "t1",
                    &IntentWrite {
                        desired_state: SandboxDesiredState::Stopped,
                        stop_reason: Some(SandboxStopReason::User),
                        retention: None,
                        marker: marker(OperationKind::Stop, 1),
                    },
                )
                .await
                .unwrap()
        });
        assert!(
            waits_for_a_lock(&pool, &write).await.unwrap(),
            "the intent write did not wait for the `sandboxes` row, so this \
             proves nothing about the order it takes its locks in"
        );

        assert!(
            matches!(
                probe_counter(&pool, "t2").await.unwrap(),
                Probe::Recorded(_)
            ),
            "the intent write was holding the deployment's counter while it \
             waited for the `sandboxes` row"
        );

        holder.rollback().await.unwrap();
        assert_eq!(write.await.unwrap(), IntentEffect::Changed);
    }
}
