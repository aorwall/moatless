-- Add channel origin metadata to task_messages so the frontend can
-- visually distinguish channel-originated messages from user-typed ones.
ALTER TABLE task_messages ADD COLUMN channel_provider TEXT;
ALTER TABLE task_messages ADD COLUMN channel_source_name TEXT;
