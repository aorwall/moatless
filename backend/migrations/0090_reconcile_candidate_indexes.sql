-- Reconciler candidate scan indexes.
--
-- The desired-state reconciler enumerates its candidates from the DB once per
-- tick: participating Tasks that desire Running (which the fleet observation may
-- find down) and Tasks with an incomplete operation marker (a crashed
-- operation). Both scans stay cheap with partial indexes rather than walking the
-- whole tasks table.
CREATE INDEX IF NOT EXISTS tasks_desired_running_idx
    ON tasks (status)
    WHERE sandbox_desired_state = 'running';

CREATE INDEX IF NOT EXISTS tasks_incomplete_operation_idx
    ON tasks (task_id)
    WHERE sandbox_operation_generation IS NOT NULL;
