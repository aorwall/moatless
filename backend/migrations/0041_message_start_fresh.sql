-- Per-user-message flag indicating the next agent run must NOT resume the
-- prior session. Distinct from `will_not_resume`, which lives on the result
-- row and means "no future message can resume me". `start_fresh` is the
-- *user* declaring intent up front: skip `--resume`, start a new session.
ALTER TABLE task_messages
    ADD COLUMN IF NOT EXISTS start_fresh BOOLEAN NOT NULL DEFAULT FALSE;
