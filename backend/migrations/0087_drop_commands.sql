-- Remove the commands concept: the DB-backed slash-command table and the
-- message tag that recorded which command sent a message. Skills replace commands.
DROP TABLE IF EXISTS commands CASCADE;
ALTER TABLE task_messages DROP COLUMN IF EXISTS command_name;
