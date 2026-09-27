-- `own_from_seq` also marks a frozen Task's first session file: the lines
-- before it predate the freeze, and its frozen Messages already show them.
-- Such a session has no `transcoded_from_event_id`.
ALTER TABLE trajectory_sessions
    DROP CONSTRAINT IF EXISTS trajectory_sessions_own_from_seq_check;
