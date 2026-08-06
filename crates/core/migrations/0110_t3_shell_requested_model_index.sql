-- no-transaction
-- Keep each concurrent build in its own non-transactional migration.

CREATE INDEX CONCURRENTLY IF NOT EXISTS task_messages_turn_requested_model_idx
    ON task_messages (task_id, turn_number, "timestamp", id)
    INCLUDE (model)
    WHERE model IS NOT NULL;
