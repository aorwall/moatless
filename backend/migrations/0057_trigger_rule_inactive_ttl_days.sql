-- Per-trigger inactivity TTL override. NULL means "inherit the global default"
-- (env var TASK_INACTIVE_TIMEOUT_DAYS, currently 7). The reaper reads
-- tasks.inactive_ttl_days; this column flows into that value at task creation
-- time for trigger-spawned tasks.
ALTER TABLE trigger_rules ADD COLUMN inactive_ttl_days INTEGER;
