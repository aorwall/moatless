-- One change per live Workspace, so a T3 client still resuming from a cache
-- older than 0239 is replayed every project it may lack. Its listing skipped
-- Workspaces created before 0239 recorded them, so a thread in one showed no
-- project and could not take a message. The release alone does not reload the
-- cache: the descriptor's `serverVersion` is fixed and it sends no migration.
WITH live AS (
    SELECT id, row_number() OVER (ORDER BY created_at, id) AS ordinal
    FROM workspaces
    WHERE deleted IS NULL OR deleted = false
),
allocated AS (
    UPDATE task_listing_position
       SET position = position + (SELECT count(*) FROM live)
     RETURNING position
)
INSERT INTO workspace_listing_changes (position, workspace_id)
SELECT allocated.position - (SELECT count(*) FROM live) + live.ordinal, live.id
FROM live, allocated;
