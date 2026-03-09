-- Task events table: persists all sandbox, server, and agent lifecycle events.
-- Note: table may already exist (created by Drizzle migration) with slightly
-- different column names. We add missing columns and create indexes safely.
CREATE TABLE IF NOT EXISTS task_events (
    id              bigserial    PRIMARY KEY,
    event_id        text         NOT NULL,
    task_id         text         NOT NULL,
    event_type      text         NOT NULL,
    sub_type        text         NOT NULL,
    is_error        boolean      NOT NULL DEFAULT false,
    agent_type      text,
    turn_number     integer,
    payload         jsonb        NOT NULL DEFAULT '{}',
    timestamp       text         NOT NULL,
    created_at      timestamptz  NOT NULL DEFAULT now()
);

-- Add columns that may be missing from the Drizzle-created table.
ALTER TABLE task_events ADD COLUMN IF NOT EXISTS event_type text;
ALTER TABLE task_events ADD COLUMN IF NOT EXISTS agent_type text;
ALTER TABLE task_events ADD COLUMN IF NOT EXISTS turn_number integer;
ALTER TABLE task_events ADD COLUMN IF NOT EXISTS timestamp text;

CREATE INDEX IF NOT EXISTS idx_task_events_task_id ON task_events (task_id, id);
CREATE INDEX IF NOT EXISTS idx_task_events_event_id ON task_events (event_id);
