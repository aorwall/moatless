-- Add updated_at column to tasks for proper "last update" sorting
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS updated_at timestamptz DEFAULT NOW();

-- Backfill from existing timestamp columns
UPDATE tasks SET updated_at = COALESCE(last_message_at::timestamptz, created_at_db, NOW());

CREATE INDEX IF NOT EXISTS tasks_updated_at_idx ON tasks USING btree (updated_at);
