-- Move durable Sandbox lifecycle state off `tasks` onto the `sandboxes` table.
--
-- Every existing Task gets a Sandbox row carrying its current desired state,
-- stop reason, intent timestamp, and start timestamp (faithful history). Tasks
-- created after this migration follow the lazy-absence model: no row until a
-- lifecycle operation writes one. The four columns then leave `tasks`; their
-- CHECK constraints drop with them.
INSERT INTO sandboxes (
    task_id, desired_state, stop_reason, desired_state_updated_at, started_at
)
SELECT
    task_id,
    sandbox_desired_state,
    sandbox_stop_reason,
    sandbox_desired_state_updated_at,
    sandbox_started_at
FROM tasks;

ALTER TABLE tasks
    DROP COLUMN sandbox_desired_state,
    DROP COLUMN sandbox_stop_reason,
    DROP COLUMN sandbox_desired_state_updated_at,
    DROP COLUMN sandbox_started_at;
