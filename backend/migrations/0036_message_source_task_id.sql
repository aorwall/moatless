-- Track which task sent this message, for UI attribution and future queries.
ALTER TABLE task_messages ADD COLUMN source_task_id TEXT;
