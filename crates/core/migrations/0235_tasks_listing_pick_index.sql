-- no-transaction
-- The Task listing's `picked` CTE: the newest Tasks at a status, read from the
-- index alone. `INCLUDE` holds every column the access filter reads; the
-- follow join probes `task_follows_pkey`.

CREATE INDEX CONCURRENTLY IF NOT EXISTS tasks_listing_pick_idx
    ON tasks (status, updated_at DESC NULLS LAST, task_id)
    INCLUDE (visibility, scope, owner_user_id, created_by);
