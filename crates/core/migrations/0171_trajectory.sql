-- The trajectory: our own record of every Claude Code and Codex Session a Task
-- runs, one immutable event per line of the harness session file.
--
-- Deliberately no foreign key to `tasks` or to any message table: a fork's
-- chain runs through its source Task's events, and `task_forks` already lets a
-- fork outlive its source. Events are never updated or deleted.
CREATE TABLE IF NOT EXISTS trajectory_sessions (
    id                   uuid PRIMARY KEY,
    task_id              text NOT NULL,
    harness              text NOT NULL CHECK (harness IN ('claude-code', 'codex')),
    harness_session_id   text NOT NULL UNIQUE,
    forked_from_event_id uuid,
    last_event_id        uuid,
    -- -1 for an empty root; the cut's seq for a fork session with no own line.
    last_seq             bigint NOT NULL DEFAULT -1,
    created_at           timestamptz NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS trajectory_sessions_task_id_created_at_idx
    ON trajectory_sessions (task_id, created_at);

-- `seq` counts the ancestors' lines, so a fork's first own line is the cut's
-- seq + 1. `payload` is text rather than jsonb: jsonb reorders keys, and a
-- rebuilt session file must equal the original.
CREATE TABLE IF NOT EXISTS trajectory_events (
    id              uuid PRIMARY KEY,
    session_id      uuid NOT NULL REFERENCES trajectory_sessions (id),
    seq             bigint NOT NULL,
    parent_event_id uuid REFERENCES trajectory_events (id),
    kind            text NOT NULL,
    harness_item_id text,
    harness_turn_id text,
    turn_number     integer,
    payload         text NOT NULL,
    content         jsonb NOT NULL,
    created_at      timestamptz NOT NULL DEFAULT NOW(),
    UNIQUE (session_id, seq)
);

CREATE INDEX IF NOT EXISTS trajectory_events_session_item_idx
    ON trajectory_events (session_id, harness_item_id);

CREATE INDEX IF NOT EXISTS trajectory_events_session_turn_idx
    ON trajectory_events (session_id, harness_turn_id);

ALTER TABLE trajectory_sessions
    ADD CONSTRAINT trajectory_sessions_forked_from_event_fkey
        FOREIGN KEY (forked_from_event_id) REFERENCES trajectory_events (id),
    ADD CONSTRAINT trajectory_sessions_last_event_fkey
        FOREIGN KEY (last_event_id) REFERENCES trajectory_events (id);
