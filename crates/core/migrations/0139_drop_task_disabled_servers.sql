-- =========================================================================
-- Drop tasks.disabled_servers
-- =========================================================================
-- The column held the per-task set of servers auto-disabled after a crash
-- loop. Nothing writes it and nothing reads it: a crash-looping server now
-- surfaces as a `server`/`Failed` event and is provisioned again on the next
-- sandbox start.
--
-- Not reversible. Deploy the code that stopped using the column first — a
-- rollback in the window between this migration and that deploy is code-only.

ALTER TABLE tasks DROP COLUMN IF EXISTS disabled_servers;
