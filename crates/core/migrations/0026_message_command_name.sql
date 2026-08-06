-- Add command_name column to track which command was used to send a message.
ALTER TABLE task_messages ADD COLUMN command_name TEXT;
