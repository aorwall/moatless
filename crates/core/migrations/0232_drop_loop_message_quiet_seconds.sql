-- No Loop ever set `message_quiet_seconds`, and nothing reads it since #1419.
ALTER TABLE loop_configs DROP COLUMN IF EXISTS message_quiet_seconds;
