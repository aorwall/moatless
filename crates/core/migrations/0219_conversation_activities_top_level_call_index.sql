-- no-transaction
-- The Task listing's `pending` lateral: a Task's newest top-level tool call.
-- Through `(task_id, position)` it fetches every thinking block, context
-- reading, sandbox status line and subagent call that came after it.

CREATE INDEX CONCURRENTLY IF NOT EXISTS conversation_activities_top_level_call_idx
    ON conversation_activities (task_id, "position")
    INCLUDE (message_uuid)
    WHERE parent_tool_use_id IS NULL AND kind = 'tool_call';
