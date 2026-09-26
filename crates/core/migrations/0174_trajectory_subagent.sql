-- A subagent session: the parent's tool-call event that spawned it. A
-- subagent never forks, so it carries no other lineage link.
ALTER TABLE trajectory_sessions
    ADD COLUMN IF NOT EXISTS parent_event_id uuid REFERENCES trajectory_events (id),
    ADD CONSTRAINT trajectory_sessions_subagent_lineage_check
        CHECK (parent_event_id IS NULL OR forked_from_event_id IS NULL);
