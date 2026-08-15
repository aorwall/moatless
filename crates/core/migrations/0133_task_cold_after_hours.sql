-- Per-task override of the cold-storage clock: reclaim the Deployment and PVC
-- once the sandbox has been Stopped this many hours, leaving the S3 snapshot as
-- the only copy.
--
-- Nullable, unlike `inactive_ttl_days INTEGER NOT NULL DEFAULT 7` (0050): a
-- NOT NULL DEFAULT bakes the value into every existing row, so a later change to
-- SANDBOX_COLD_AFTER_HOURS would reach no task that already exists. NULL means
-- "follow the deployment default".
ALTER TABLE tasks ADD COLUMN cold_after_hours INTEGER;
