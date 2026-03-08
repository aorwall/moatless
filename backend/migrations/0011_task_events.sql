-- Task events table: persists all sandbox, server, and agent lifecycle events.
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

CREATE INDEX idx_task_events_task_id ON task_events (task_id, id);
CREATE INDEX idx_task_events_task_type ON task_events (task_id, event_type);
CREATE INDEX idx_task_events_event_id ON task_events (event_id);
