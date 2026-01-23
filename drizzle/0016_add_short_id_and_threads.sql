-- Add short_id column to tasks table for human-readable task identifiers
ALTER TABLE tasks ADD COLUMN short_id TEXT;--> statement-breakpoint
CREATE UNIQUE INDEX tasks_short_id_idx ON tasks (short_id);--> statement-breakpoint

-- Add iteration_threads table for tracking thread relationships
CREATE TABLE iteration_threads (
  id TEXT PRIMARY KEY NOT NULL,
  task_id TEXT NOT NULL REFERENCES tasks(task_id) ON DELETE CASCADE,
  iteration_id TEXT NOT NULL REFERENCES iterations(id) ON DELETE CASCADE,
  channel_type TEXT NOT NULL,
  external_thread_id TEXT NOT NULL,
  external_channel_id TEXT,
  metadata TEXT,
  created_at TEXT DEFAULT (datetime('now')) NOT NULL,
  updated_at TEXT DEFAULT (datetime('now')) NOT NULL
);--> statement-breakpoint

CREATE INDEX iteration_threads_task_id_idx ON iteration_threads (task_id);--> statement-breakpoint
CREATE INDEX iteration_threads_iteration_id_idx ON iteration_threads (iteration_id);--> statement-breakpoint
CREATE UNIQUE INDEX iteration_threads_unique_thread_idx ON iteration_threads (task_id, channel_type, external_thread_id);
