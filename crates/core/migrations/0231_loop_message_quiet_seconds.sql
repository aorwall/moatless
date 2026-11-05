-- How long a Loop's buffer window waits for the next event before it closes.
-- NULL keeps the built-in quiet period.
ALTER TABLE loop_configs ADD COLUMN IF NOT EXISTS message_quiet_seconds integer;
