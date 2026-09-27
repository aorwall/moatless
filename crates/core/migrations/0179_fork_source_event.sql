-- A fork is anchored by `source_event_id` alone. The retired `task_message*`
-- and `task_activities` tables stay until a later migration drops them.
ALTER TABLE task_forks
    DROP COLUMN IF EXISTS source_session_id,
    DROP COLUMN IF EXISTS source_message_uuid,
    DROP COLUMN IF EXISTS source_agent_turn_id;

-- A row with no event predates the Trajectory; the anchors that resolved it
-- are gone, so its next turn starts a fresh session.
DELETE FROM task_forks WHERE source_event_id IS NULL;
ALTER TABLE task_forks ALTER COLUMN source_event_id SET NOT NULL;
