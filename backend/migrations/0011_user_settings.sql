-- Add settings JSONB column to users table for user preferences
-- (e.g., default_channel_repository_id for auto-created channel configs).
ALTER TABLE users ADD COLUMN IF NOT EXISTS settings jsonb NOT NULL DEFAULT '{}';
