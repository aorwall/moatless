-- no-transaction
-- Keep the marker above as byte zero: SQLx uses it to permit CONCURRENTLY.
--
-- `tasks_listing_pick_idx` plus `reporter_user_id`, which the Task access
-- filter now reads; without it the listing's `picked` CTE reads the heap.
-- 0238 drops the old index: one concurrent build per migration.

CREATE INDEX CONCURRENTLY IF NOT EXISTS tasks_listing_pick_reporter_idx
    ON tasks (status, updated_at DESC NULLS LAST, task_id)
    INCLUDE (visibility, scope, owner_user_id, created_by, reporter_user_id);
