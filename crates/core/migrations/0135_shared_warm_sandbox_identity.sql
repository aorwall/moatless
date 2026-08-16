-- Give Sandboxes their own identity while preserving every existing runtime
-- resource name. The backfill deliberately copies task_id into sandbox_id.

ALTER TABLE sandboxes ADD COLUMN IF NOT EXISTS sandbox_id text;
ALTER TABLE sandboxes ADD COLUMN IF NOT EXISTS workspace_id text;

UPDATE sandboxes s
SET sandbox_id = s.task_id,
    workspace_id = COALESCE(t.workspace_id, 'ws_' || t.repository_id)
FROM tasks t
WHERE s.task_id = t.task_id
  AND (s.sandbox_id IS NULL OR s.workspace_id IS NULL);

ALTER TABLE tasks ADD COLUMN IF NOT EXISTS sandbox_id text;
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS checkout_mode text;

UPDATE tasks t
SET sandbox_id = s.sandbox_id
FROM sandboxes s
WHERE s.task_id = t.task_id
  AND t.sandbox_id IS NULL;

DROP INDEX IF EXISTS sandboxes_desired_running_idx;
DROP INDEX IF EXISTS sandboxes_incomplete_operation_idx;
DROP INDEX IF EXISTS sandboxes_reclaimable_idx;

ALTER TABLE sandboxes DROP CONSTRAINT IF EXISTS sandboxes_task_id_fkey;
ALTER TABLE sandboxes DROP CONSTRAINT IF EXISTS sandboxes_pkey;

ALTER TABLE sandboxes
    ALTER COLUMN sandbox_id SET NOT NULL,
    ALTER COLUMN workspace_id SET NOT NULL,
    ADD CONSTRAINT sandboxes_pkey PRIMARY KEY (sandbox_id);

ALTER TABLE sandboxes DROP COLUMN IF EXISTS task_id;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'sandboxes_workspace_id_fkey'
    ) THEN
        ALTER TABLE sandboxes
            ADD CONSTRAINT sandboxes_workspace_id_fkey
            FOREIGN KEY (workspace_id) REFERENCES workspaces(id) ON DELETE RESTRICT;
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'tasks_sandbox_id_fkey'
    ) THEN
        ALTER TABLE tasks
            ADD CONSTRAINT tasks_sandbox_id_fkey
            FOREIGN KEY (sandbox_id) REFERENCES sandboxes(sandbox_id) ON DELETE SET NULL;
    END IF;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'tasks_checkout_mode_check'
    ) THEN
        ALTER TABLE tasks
            ADD CONSTRAINT tasks_checkout_mode_check
            CHECK (checkout_mode IS NULL OR checkout_mode IN ('shared', 'isolated'));
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS tasks_sandbox_id_idx
    ON tasks (sandbox_id);

CREATE INDEX IF NOT EXISTS sandboxes_desired_running_idx
    ON sandboxes (sandbox_id)
    WHERE desired_state = 'running';

CREATE INDEX IF NOT EXISTS sandboxes_incomplete_operation_idx
    ON sandboxes (sandbox_id)
    WHERE operation_generation IS NOT NULL;

CREATE INDEX IF NOT EXISTS sandboxes_reclaimable_idx
    ON sandboxes (desired_state_updated_at)
    WHERE desired_state = 'stopped' AND storage_reclaimed_at IS NULL;
