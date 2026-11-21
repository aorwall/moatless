-- `site_url`, `site_id` and `site_provider` were never set (0 of 4,732 tasks in a month) and
-- `vibe` is stored and echoed but read by nothing.
ALTER TABLE tasks DROP COLUMN IF EXISTS site_url;
ALTER TABLE tasks DROP COLUMN IF EXISTS site_id;
ALTER TABLE tasks DROP COLUMN IF EXISTS site_provider;
ALTER TABLE tasks DROP COLUMN IF EXISTS vibe;
