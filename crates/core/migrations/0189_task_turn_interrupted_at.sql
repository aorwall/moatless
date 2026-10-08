-- When a turn became `interrupted`. The reaper closes a turn that stayed
-- interrupted past its recovery window as `error`. NULL unless `interrupted`.
ALTER TABLE task_turns
    ADD COLUMN IF NOT EXISTS interrupted_at timestamptz;
