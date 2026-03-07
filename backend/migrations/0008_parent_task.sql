ALTER TABLE tasks ADD COLUMN IF NOT EXISTS parent_task_id TEXT;
CREATE INDEX IF NOT EXISTS tasks_parent_task_id_idx ON tasks USING btree (parent_task_id);
