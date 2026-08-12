-- The listing change log names Tasks only. Workspace state is projected into
-- snapshots and while re-reading changed Tasks, but Workspace mutations do not
-- advance the live cursor.
--
-- Deleting Workspace records can leave gaps below the preserved counter. Gaps
-- are safe: readers order the rows that remain and resume eligibility is based
-- on the counter, so a gap can only make a client conservatively rebuild.
DELETE FROM task_listing_changes WHERE workspace_id IS NOT NULL;

ALTER TABLE task_listing_changes
    DROP CONSTRAINT IF EXISTS task_listing_changes_names_one_subject;
ALTER TABLE task_listing_changes DROP COLUMN workspace_id;
ALTER TABLE task_listing_changes ALTER COLUMN task_id SET NOT NULL;
