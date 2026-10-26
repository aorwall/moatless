-- When the viewer opted this Task out of the reaper's auto-settle; NULL while
-- the reaper may settle it for them.
ALTER TABLE task_follows ADD COLUMN IF NOT EXISTS auto_settle_disabled_at TIMESTAMPTZ;

-- When the snooze `thread.unsnooze` cleared woke: by hand, or on its timer if
-- that ran out first. The reaper counts it as activity.
ALTER TABLE task_follows ADD COLUMN IF NOT EXISTS snooze_woke_at TIMESTAMPTZ;
