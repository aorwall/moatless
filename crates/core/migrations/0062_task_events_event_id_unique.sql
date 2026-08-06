-- task_events.event_id is a backend-generated UUIDv4 stamped at XADD time, so
-- it is globally unique. A unique index lets the event-handler consumer insert
-- idempotently (ON CONFLICT (event_id) DO NOTHING) when it re-delivers a
-- pending entry that was processed but not ACKed before a Redis disconnect.

-- Deduplicate any rows that share an event_id, keeping the earliest (lowest
-- id), so the unique-index creation below cannot fail on historical duplicates.
DELETE FROM task_events a
USING task_events b
WHERE a.event_id = b.event_id
  AND a.id > b.id;

DROP INDEX IF EXISTS idx_task_events_event_id;
CREATE UNIQUE INDEX IF NOT EXISTS idx_task_events_event_id ON task_events (event_id);
