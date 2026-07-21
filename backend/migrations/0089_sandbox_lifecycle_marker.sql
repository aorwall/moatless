-- Durable correlation marker + archive-retention for the sandbox lifecycle module.
--
-- The lifecycle module writes desired state, stop reason, archive retention, and
-- an operation correlation marker (kind + generation + started_at) in one atomic
-- statement. The marker outlives the runtime call so late watcher events can be
-- correlated to the operation that caused them; the reconciler is its
-- acknowledgement owner. `sandbox_operation_seq` is the durable per-Task
-- allocation counter — a new operation allocates the next generation, recovery
-- reuses the marker's generation.
--
-- "operation_generation" is qualified to avoid colliding with Kubernetes resource
-- generations.
ALTER TABLE tasks
    ADD COLUMN IF NOT EXISTS sandbox_archive_retention text,
    ADD COLUMN IF NOT EXISTS sandbox_operation_kind text,
    ADD COLUMN IF NOT EXISTS sandbox_operation_generation bigint,
    ADD COLUMN IF NOT EXISTS sandbox_operation_started_at timestamptz,
    ADD COLUMN IF NOT EXISTS sandbox_operation_seq bigint NOT NULL DEFAULT 0;

ALTER TABLE tasks
    ADD CONSTRAINT tasks_sandbox_archive_retention_check
    CHECK (
        sandbox_archive_retention IS NULL
        OR sandbox_archive_retention IN ('retain_archives', 'delete_archives')
    ) NOT VALID;

ALTER TABLE tasks
    ADD CONSTRAINT tasks_sandbox_operation_kind_check
    CHECK (
        sandbox_operation_kind IS NULL
        OR sandbox_operation_kind IN ('start', 'stop', 'restart', 'redeploy', 'remove')
    ) NOT VALID;

ALTER TABLE tasks VALIDATE CONSTRAINT tasks_sandbox_archive_retention_check;
ALTER TABLE tasks VALIDATE CONSTRAINT tasks_sandbox_operation_kind_check;

-- Reconciliation of Tasks with unacknowledged user messages must not scan the
-- message table one Task at a time. A pending user message is
-- `agent_session_id IS NULL AND message_type = 'user'`; this partial index keeps
-- the cross-Task pending scan cheap.
CREATE INDEX IF NOT EXISTS task_messages_pending_user_idx
    ON task_messages (task_id)
    WHERE agent_session_id IS NULL AND message_type = 'user';
