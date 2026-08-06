-- Add agent_type column to channel_configs
-- Allows configuring which agent type (claude-code, codex, etc.) to use for tasks created from this channel
ALTER TABLE channel_configs ADD COLUMN agent_type TEXT NOT NULL DEFAULT 'claude-code';
