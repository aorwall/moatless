-- A Task has no scope: `visibility`, the Reporter and the owner
-- (`COALESCE(owner_user_id, created_by)`) decide who reaches it, and 0214 moved
-- every Task with a creator to `user`. Its Sandbox carried the same column only
-- to mirror the Task, so both lose it.
--
-- A Task with no creator and no owner stayed `global` after 0214. It keeps its
-- row, `visibility` and (absent) owner: private, it is now reachable by admins
-- only, where `global` let every signed-in user read it; public, it is
-- unchanged. Narrowing the read is deliberate, since the alternative of
-- setting `visibility = 'public'` would grant every signed-in user write.

-- Dropping the column drops these with it; named so the intent is on the page.
-- The covering index lists `scope`, so it is rebuilt without it, plain rather
-- than concurrent because a transactional migration cannot build concurrently
-- and `tasks` is small enough to index in a moment. 0246's `(scope, owner_user_id)`
-- index is not replaced: no query tests `owner_user_id` bare, since the access
-- rule reads `COALESCE(owner_user_id, created_by)` inside an `OR`.
DROP INDEX IF EXISTS tasks_listing_pick_reporter_idx;
DROP INDEX IF EXISTS tasks_scope_owner_idx;
ALTER TABLE tasks DROP CONSTRAINT IF EXISTS tasks_owner_scope_chk;
ALTER TABLE sandboxes DROP CONSTRAINT IF EXISTS sandboxes_scope_check;

ALTER TABLE tasks DROP COLUMN scope;
ALTER TABLE sandboxes DROP COLUMN scope;

CREATE INDEX tasks_listing_pick_reporter_idx
    ON tasks (status, updated_at DESC NULLS LAST, task_id)
    INCLUDE (visibility, owner_user_id, created_by, reporter_user_id);
