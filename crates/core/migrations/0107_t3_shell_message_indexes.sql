-- no-transaction
-- Keep the marker above as byte zero: SQLx uses it to permit CONCURRENTLY.
--
-- A full T3 shell snapshot derives the latest user message for every followed
-- task. The original generic `(task_id, timestamp, id)` index left PostgreSQL
-- scanning and sorting the task's whole message history.
--
-- Keep one concurrent build per migration. PostgreSQL treats a multi-statement
-- protocol message as a transaction block even when SQLx does not start one.

CREATE INDEX CONCURRENTLY IF NOT EXISTS task_messages_latest_user_created_idx
    ON task_messages (task_id, created_at DESC)
    WHERE message_type = 'user';
