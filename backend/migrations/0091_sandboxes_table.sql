-- Durable per-Task Sandbox lifecycle state.
--
-- One row per Task holds the desired state, stop reason, intent timestamp,
-- start timestamp, archive retention, the operation correlation marker
-- (kind + generation + started_at), and the per-Task generation counter. The
-- lifecycle module is the sole writer: the atomic intent write is an
-- `INSERT ... ON CONFLICT DO UPDATE`, so the row is born lazily and only through
-- a lifecycle operation. Absence of a row means no Sandbox was ever requested —
-- converged with observed absence by definition.
--
-- The marker outlives the runtime call so late watcher events can be correlated
-- to the operation that caused them; the reconciler is its acknowledgement
-- owner. `operation_seq` is the durable per-Task allocation counter — a new
-- operation allocates the next generation, recovery reuses the marker's.
-- "operation_generation" is qualified to avoid colliding with Kubernetes
-- resource generations.
--
-- The row is deleted only with its Task (`ON DELETE CASCADE`), which is safe
-- because Task deletion is gated on strict removal success.
CREATE TABLE IF NOT EXISTS sandboxes (
    task_id text PRIMARY KEY REFERENCES tasks(task_id) ON DELETE CASCADE,
    desired_state text NOT NULL DEFAULT 'stopped',
    stop_reason text,
    desired_state_updated_at timestamptz NOT NULL DEFAULT NOW(),
    started_at timestamptz,
    archive_retention text,
    operation_kind text,
    operation_generation bigint,
    operation_started_at timestamptz,
    operation_seq bigint NOT NULL DEFAULT 0,
    updated_at timestamptz NOT NULL DEFAULT NOW(),
    CONSTRAINT sandboxes_desired_state_check
        CHECK (desired_state IN ('running', 'stopped', 'removed')),
    CONSTRAINT sandboxes_stop_reason_check
        CHECK (
            stop_reason IS NULL
            OR stop_reason IN ('user', 'idle_reaper', 'cleanup', 'system', 'error')
        ),
    CONSTRAINT sandboxes_archive_retention_check
        CHECK (
            archive_retention IS NULL
            OR archive_retention IN ('retain_archives', 'delete_archives')
        ),
    CONSTRAINT sandboxes_operation_kind_check
        CHECK (
            operation_kind IS NULL
            OR operation_kind IN ('start', 'stop', 'restart', 'redeploy', 'remove')
        )
);

-- Reconciliation of Tasks with unacknowledged user messages must not scan the
-- message table one Task at a time. A pending user message is
-- `agent_session_id IS NULL AND message_type = 'user'`; this partial index keeps
-- the cross-Task pending scan cheap.
CREATE INDEX IF NOT EXISTS task_messages_pending_user_idx
    ON task_messages (task_id)
    WHERE agent_session_id IS NULL AND message_type = 'user';
