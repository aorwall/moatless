-- Deactivate a task's bindings when the task closes, instead of leaving them
-- live forever. A closed task is terminal (no reopen path), so its external
-- "listeners" (Linear issue / GitHub PR / Slack thread bindings) must stop
-- routing inbound comments — otherwise a new comment re-provisions the sandbox
-- and resurrects a dead task.
--
-- Rows are kept (not deleted) so the admin Task Connections view still shows a
-- closed task's connections, flagged inactive. Inbound routing filters on
-- `active`; the close path flips it to FALSE and stamps `deactivated_at`.

ALTER TABLE task_bindings
    ADD COLUMN IF NOT EXISTS active boolean NOT NULL DEFAULT true,
    ADD COLUMN IF NOT EXISTS deactivated_at timestamptz;

-- Backfill: deactivate bindings whose task is already terminal. `tasks` keys
-- bindings by the business `task_id` (text), not the serial `id`. Status is
-- stored as lowercase text (see TaskStatus::as_str).
UPDATE task_bindings b
SET active = false,
    deactivated_at = now()
FROM tasks t
WHERE b.task_id = t.task_id
  AND t.status IN ('closed', 'error')
  AND b.active = true;

-- Inbound routing looks up bindings by subject and then filters active; index
-- the active flag alongside the existing subject index path.
CREATE INDEX IF NOT EXISTS "task_bindings_active_idx"
    ON "task_bindings" ("active");
