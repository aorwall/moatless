-- Persist task-owned sandbox lifecycle intent.
--
-- `sandbox_desired_state` is the application-level target state, distinct from
-- observed infrastructure status. Recovery uses it to avoid replaying an
-- interrupted turn after an intentional stop or cleanup.
ALTER TABLE tasks
    ADD COLUMN IF NOT EXISTS sandbox_desired_state text NOT NULL DEFAULT 'running',
    ADD COLUMN IF NOT EXISTS sandbox_stop_reason text,
    ADD COLUMN IF NOT EXISTS sandbox_desired_state_updated_at timestamptz NOT NULL DEFAULT NOW();

ALTER TABLE tasks
    ADD CONSTRAINT tasks_sandbox_desired_state_check
    CHECK (sandbox_desired_state IN ('running', 'stopped', 'removed')) NOT VALID;

ALTER TABLE tasks
    ADD CONSTRAINT tasks_sandbox_stop_reason_check
    CHECK (
        sandbox_stop_reason IS NULL
        OR sandbox_stop_reason IN ('user', 'idle_reaper', 'cleanup', 'system', 'error')
    ) NOT VALID;

ALTER TABLE tasks VALIDATE CONSTRAINT tasks_sandbox_desired_state_check;
ALTER TABLE tasks VALIDATE CONSTRAINT tasks_sandbox_stop_reason_check;
