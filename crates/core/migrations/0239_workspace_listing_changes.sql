-- The durable record that a Workspace's project-visible state changed, at a
-- position from the same `task_listing_position` counter as Task changes.
--
-- A T3 client resumes from one position and is replayed every change after it,
-- projects included (`project.updated`/`project.removed`). Without a record, a
-- client whose cached listing predates a Workspace never learns it exists, and
-- threads in it cannot be sent anywhere.
--
-- A table of its own rather than a subject column on `task_listing_changes`
-- (0105, dropped by 0130): a replica that predates this reads that table with
-- `task_id NOT NULL` and keeps working through a rolling deploy.
--
-- No foreign key: the Workspace going away is itself a change a resuming
-- client needs, so the record must outlive the row.
CREATE TABLE IF NOT EXISTS workspace_listing_changes (
    position     bigint      PRIMARY KEY,
    workspace_id text        NOT NULL,
    recorded_at  timestamptz NOT NULL DEFAULT NOW()
);
