-- The turns a rewind cut out of a Task, kept so a reader can show them as
-- rolled back. `task_turns` holds only the conversation's live turns; a rewind
-- moves its rows here instead of deleting them (`task/rewind_dao.rs`), and the
-- next turn is numbered above both tables, so a turn number is never reused.
CREATE TABLE IF NOT EXISTS task_rewound_turns (
    task_id      TEXT        NOT NULL REFERENCES tasks(task_id) ON DELETE CASCADE,
    turn_number  INTEGER     NOT NULL,
    run_id       TEXT,
    -- The `task_turns.status` the turn ended with before the rewind.
    status       TEXT        NOT NULL,
    started_at   TIMESTAMPTZ NOT NULL,
    ended_at     TIMESTAMPTZ,
    rewound_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (task_id, turn_number)
);
