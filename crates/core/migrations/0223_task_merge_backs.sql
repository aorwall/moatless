CREATE TABLE task_merge_backs (
    target_task_id text NOT NULL REFERENCES tasks(task_id) ON DELETE CASCADE,
    -- A queued summary survives deletion of its source conversation.
    source_task_id text NOT NULL,
    through_turn integer NOT NULL CHECK (through_turn > 0),
    summary text NOT NULL,
    pending boolean NOT NULL DEFAULT true,
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (target_task_id, source_task_id)
);
