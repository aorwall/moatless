-- `task_forks` also carries a rewind, so two of its columns mean something
-- wider than 0150 wrote. Restated here rather than edited there, because
-- `sqlx::migrate!` checksums an applied file — the same reason 0124 restates a
-- comment from 0122.
--
-- A rewind (`task/rewind.rs`) deletes a Task's turns above a cut and leaves the
-- Task resuming its own agent session branched at that cut, which is the
-- delivery path's fork intent exactly. So a row here is "the session this Task's
-- next turn branches from", and the Task is its own source when the row records
-- a rewind.
COMMENT ON COLUMN task_forks.source_task_id IS
    'The Task the session is branched from: the parent for a fork, the Task itself for a rewind. No foreign key, so it survives the parent being deleted.';
COMMENT ON COLUMN task_forks.at_turn IS
    'The source turn the cut keeps. NULL only for a fork at head; a rewind always names one.';
