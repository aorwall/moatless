-- Per-task list of server names that have been auto-disabled after repeatedly
-- crash-looping. Stored per-task (not on repositories.servers) so disabling a
-- broken server for one task does not affect other tasks on the same repo.
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS disabled_servers JSONB;
