ALTER TABLE channel_configs ADD COLUMN IF NOT EXISTS task_name text;
ALTER TABLE channel_configs ADD COLUMN IF NOT EXISTS tag_ids text[] NOT NULL DEFAULT '{}';
