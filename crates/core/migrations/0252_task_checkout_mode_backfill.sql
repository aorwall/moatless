-- A NULL `checkout_mode` has always been read as shared. Backfill it and
-- default it; NOT NULL waits for a release that never writes NULL, because
-- the previous release still binds an explicit NULL on create.
UPDATE tasks SET checkout_mode = 'shared' WHERE checkout_mode IS NULL;
ALTER TABLE tasks ALTER COLUMN checkout_mode SET DEFAULT 'shared';
