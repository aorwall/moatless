-- no-transaction
-- Keep each concurrent build in its own non-transactional migration.

CREATE INDEX CONCURRENTLY IF NOT EXISTS task_messages_latest_assistant_idx
    ON task_messages (task_id, id DESC)
    WHERE message_type = 'assistant';
