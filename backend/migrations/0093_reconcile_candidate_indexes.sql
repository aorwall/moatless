-- Reconciler candidate scan indexes on `sandboxes`.
--
-- The desired-state reconciler enumerates its candidates from the DB once per
-- tick: Sandboxes that desire Running (which the fleet observation may find
-- down) and Sandboxes with an incomplete operation marker (a crashed
-- operation). Both scans stay cheap with partial indexes rather than walking the
-- whole table. The candidate query joins each hit to `tasks` by primary key to
-- filter on participating (Open/Error) status.
CREATE INDEX IF NOT EXISTS sandboxes_desired_running_idx
    ON sandboxes (task_id)
    WHERE desired_state = 'running';

CREATE INDEX IF NOT EXISTS sandboxes_incomplete_operation_idx
    ON sandboxes (task_id)
    WHERE operation_generation IS NOT NULL;
