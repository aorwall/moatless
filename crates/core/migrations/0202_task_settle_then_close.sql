-- Auto-close counts from settling rather than from the last activity: the
-- reaper settles a task idle past `settle_after_hours`, then closes it
-- `close_after_settled_days` after it was settled.
ALTER TABLE tasks RENAME COLUMN inactive_ttl_days TO close_after_settled_days;
ALTER TABLE loop_configs RENAME COLUMN inactive_ttl_days TO close_after_settled_days;

-- NULL follows the deployment default (TASK_SETTLE_AFTER_HOURS), as
-- `cold_after_hours` does (0133).
ALTER TABLE tasks ADD COLUMN settle_after_hours INTEGER;
-- NULL gives the loop's tasks the loop default.
ALTER TABLE loop_configs ADD COLUMN settle_after_hours INTEGER;

-- When the reaper settled the task, for every follower and for a task nobody
-- follows. Cleared with the followers' settled state on new activity.
-- Not `settled_at`: the listing selects `t.*` beside `task_follows.settled_at`.
ALTER TABLE tasks ADD COLUMN auto_settled_at TIMESTAMPTZ;

-- Open Loop tasks predate the column. Without a settle TTL a Loop task nobody
-- follows would never settle, and so never close.
UPDATE tasks SET settle_after_hours = 1 WHERE loop_id IS NOT NULL AND status = 'open';
