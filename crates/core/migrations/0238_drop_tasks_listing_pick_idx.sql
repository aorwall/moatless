-- no-transaction
-- Keep the marker above as byte zero: SQLx uses it to permit CONCURRENTLY.
-- Superseded by `tasks_listing_pick_reporter_idx` (0237).

DROP INDEX CONCURRENTLY IF EXISTS tasks_listing_pick_idx;
