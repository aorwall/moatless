-- When a task was closed. Null while it is open; set by the same write that
-- sets `status`, cleared on reopen.
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS closed_at TIMESTAMPTZ;

-- Tasks closed before this column existed get their last write as the closing
-- time. It is the nearest recorded fact, and a closed row with no time at all
-- reads as "never closed" to everything downstream.
UPDATE tasks
SET closed_at = COALESCE(updated_at, created_at_db, NOW())
WHERE status = 'closed'
  AND closed_at IS NULL;
