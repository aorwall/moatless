-- Canonical pointer to the trigger that spawned this task. Distinct from
-- task_bindings.trigger_id (which addresses a conversation/return channel) so
-- that all trigger kinds — including schedule triggers, which never create a
-- binding — are addressable for trigger-detail listings.

ALTER TABLE tasks
    ADD COLUMN IF NOT EXISTS trigger_id TEXT;

ALTER TABLE tasks
    DROP CONSTRAINT IF EXISTS tasks_trigger_id_fkey;

ALTER TABLE tasks
    ADD CONSTRAINT tasks_trigger_id_fkey
    FOREIGN KEY (trigger_id) REFERENCES triggers (id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS tasks_trigger_id_idx
    ON tasks (trigger_id)
    WHERE trigger_id IS NOT NULL;

-- Backfill: adapter-event triggers — copy from earliest matching binding.
UPDATE tasks t
SET trigger_id = sub.trigger_id
FROM (
    SELECT DISTINCT ON (task_id) task_id, trigger_id
    FROM task_bindings
    WHERE trigger_id IS NOT NULL
    ORDER BY task_id, created_at ASC
) sub
WHERE sub.task_id = t.task_id
  AND t.trigger_id IS NULL;

-- Backfill: schedule triggers — derive from the first scheduled message that
-- pinned the originating trigger id in `external_source_name`.
UPDATE tasks t
SET trigger_id = sub.trig
FROM (
    SELECT DISTINCT ON (task_id) task_id, external_source_name AS trig
    FROM task_messages
    WHERE external_adapter = 'scheduled' AND external_source_name IS NOT NULL
    ORDER BY task_id, id ASC
) sub
WHERE sub.task_id = t.task_id
  AND t.trigger_id IS NULL
  AND EXISTS (SELECT 1 FROM triggers WHERE id = sub.trig);
