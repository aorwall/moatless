-- Denormalize task_id onto child tables to avoid slow JOINs
-- The JOIN on task_message_id → filter by task_id causes 1-3s queries
-- that exhaust the connection pool under load.

-- Add task_id to task_message_contexts
ALTER TABLE task_message_contexts ADD COLUMN IF NOT EXISTS task_id VARCHAR(36);

-- Backfill from parent table
UPDATE task_message_contexts c
SET task_id = m.task_id
FROM task_messages m
WHERE c.task_message_id = m.id
  AND c.task_id IS NULL;

-- Add index for direct lookup
CREATE INDEX IF NOT EXISTS idx_task_message_contexts_task_id
ON task_message_contexts (task_id);

-- Add task_id to task_message_content_blocks (same issue)
ALTER TABLE task_message_content_blocks ADD COLUMN IF NOT EXISTS task_id VARCHAR(36);

-- Backfill from parent table
UPDATE task_message_content_blocks b
SET task_id = m.task_id
FROM task_messages m
WHERE b.task_message_id = m.id
  AND b.task_id IS NULL;

-- Add index for direct lookup
CREATE INDEX IF NOT EXISTS idx_task_message_content_blocks_task_id
ON task_message_content_blocks (task_id);
