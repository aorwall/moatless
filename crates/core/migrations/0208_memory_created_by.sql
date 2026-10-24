-- Who created a memory entry: set by the put that inserts the row and never
-- changed. A put over an expired row inserts a new row, so it sets them again.
-- Existing rows take them from their first `put` event and its Task's Loop.

ALTER TABLE memory_entries
    ADD COLUMN IF NOT EXISTS created_by_task_id text,
    ADD COLUMN IF NOT EXISTS created_by_loop_id text,
    ADD COLUMN IF NOT EXISTS created_by_user_id text;

UPDATE memory_entries e SET
    created_by_task_id = first.task_id,
    created_by_user_id = first.user_id,
    created_by_loop_id = t.loop_id
FROM (
    SELECT DISTINCT ON (entry_id) entry_id, task_id, user_id
    FROM memory_events
    WHERE op = 'put'
    ORDER BY entry_id, event_id
) first
LEFT JOIN tasks t ON t.task_id = first.task_id
WHERE first.entry_id = e.entry_id AND e.created_by_task_id IS NULL;
