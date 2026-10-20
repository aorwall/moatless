-- Up to when a Task's agent has seen its preloaded memory. A fetch stages the
-- time it read; the acknowledgement of that delivery commits it, so a note
-- that never reached the agent is reported again.
CREATE TABLE IF NOT EXISTS memory_seen (
    task_id text PRIMARY KEY,
    seen_at timestamptz,
    staged_at timestamptz
);
