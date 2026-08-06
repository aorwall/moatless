-- Backfill turn_number on task_messages and convert orphan trailing assistant
-- rows into result rows so every completed turn has a single terminal row.
--
-- Idempotent: only touches rows where turn_number IS NULL. Re-running after
-- the first execution is a no-op.

-- 1) Convert orphan trailing assistant rows into 'result'.
--
-- A row qualifies as an orphan turn boundary when:
--   * it is currently 'assistant' and has no turn_number yet, AND
--   * the next row in the same task (ordered by timestamp, id) is either a
--     'user' row or there is no next row at all.
-- We deliberately do NOT touch assistant rows that are already followed by a
-- 'result'/'error' row — those turns already have a terminal marker.
WITH ordered AS (
    SELECT
        id,
        task_id,
        message_type,
        turn_number,
        ROW_NUMBER() OVER (PARTITION BY task_id ORDER BY timestamp, id) AS rn
    FROM task_messages
),
with_next AS (
    SELECT
        o.id,
        o.task_id,
        o.message_type,
        o.turn_number,
        LEAD(o.message_type) OVER (PARTITION BY o.task_id ORDER BY o.rn) AS next_type
    FROM ordered o
)
UPDATE task_messages m
SET message_type = 'result'
FROM with_next n
WHERE m.id = n.id
  AND m.turn_number IS NULL
  AND n.message_type = 'assistant'
  AND (n.next_type IS NULL OR n.next_type = 'user');

-- 2) Backfill turn_number per task.
--
-- For each row, count how many 'result'/'error' rows occurred strictly before
-- it in the same task; the row's turn_number is that count + 1. This naturally
-- groups everything between two terminal rows into the same turn.
WITH ordered AS (
    SELECT
        id,
        task_id,
        message_type,
        ROW_NUMBER() OVER (PARTITION BY task_id ORDER BY timestamp, id) AS rn,
        CASE WHEN message_type IN ('result', 'error') THEN 1 ELSE 0 END AS ends_turn
    FROM task_messages
),
with_count AS (
    SELECT
        id,
        COALESCE(
            SUM(ends_turn) OVER (
                PARTITION BY task_id
                ORDER BY rn
                ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING
            ),
            0
        ) + 1 AS computed_turn
    FROM ordered
)
UPDATE task_messages m
SET turn_number = c.computed_turn
FROM with_count c
WHERE m.id = c.id
  AND m.turn_number IS NULL;

-- Helpful index for /messages/turns/{turn_number} lookups.
CREATE INDEX IF NOT EXISTS task_messages_task_turn_idx
    ON task_messages (task_id, turn_number);
