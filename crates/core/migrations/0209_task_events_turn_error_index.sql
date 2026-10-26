-- no-transaction
-- The Task listing's `failure` lateral: a Turn's latest error event. Through
-- `(task_id, id)` alone it reads every event the Task ever logged.

CREATE INDEX CONCURRENTLY IF NOT EXISTS task_events_turn_error_idx
    ON task_events (task_id, turn_number, id)
    WHERE sub_type = 'error';
