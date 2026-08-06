-- Allow multiple tasks to connect to the same external thread (e.g. PR).
-- The old index enforced (config_id, external_thread_id) uniqueness;
-- the new one enforces (config_id, external_thread_id, task_id).

DROP INDEX IF EXISTS "channel_threads_config_thread_idx";
CREATE UNIQUE INDEX "channel_threads_config_thread_task_idx"
    ON "channel_threads" ("config_id", "external_thread_id", "task_id");
