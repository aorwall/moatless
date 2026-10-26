-- no-transaction
-- The Task listing's `requested_model`: a Turn's Messages that carry an inbox
-- item. Through `(task_id, turn_number)` it fetches every agent Message too.

CREATE INDEX CONCURRENTLY IF NOT EXISTS conversation_messages_turn_inbox_idx
    ON conversation_messages (task_id, turn_number, "position")
    INCLUDE (inbox_id)
    WHERE inbox_id IS NOT NULL;
