-- Add append_system_prompt column to tasks (used by channel-created tasks to pass
-- the channel config's prompt_template as a system-level instruction to the agent).
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS append_system_prompt TEXT;
