-- no-transaction
-- Keep each concurrent build in its own non-transactional migration.

CREATE INDEX CONCURRENTLY IF NOT EXISTS task_messages_turn_assistant_model_idx
    ON task_messages (task_id, turn_number, "timestamp" DESC, id DESC)
    INCLUDE (assistant_message_model)
    WHERE assistant_message_model IS NOT NULL;
