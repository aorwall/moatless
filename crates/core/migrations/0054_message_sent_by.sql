-- Identify the moatless user that authored a message. NULL for messages
-- originated by the system (schedulers, bindings, adapters, error results).

ALTER TABLE task_messages ADD COLUMN IF NOT EXISTS sent_by TEXT;

CREATE INDEX IF NOT EXISTS task_messages_sent_by_idx
    ON task_messages (sent_by);
