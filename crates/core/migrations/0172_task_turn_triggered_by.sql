-- The task whose message opened a turn, which is the task told how that turn
-- ended. `trigger_decided` is set by the first acknowledgement, so a turn a
-- person opened stays nobody's when a task joins it. Turns before these
-- columns stay undecided and tell nobody.
ALTER TABLE task_turns ADD COLUMN IF NOT EXISTS triggered_by_task_id text;
ALTER TABLE task_turns ADD COLUMN IF NOT EXISTS trigger_decided boolean NOT NULL DEFAULT false;
