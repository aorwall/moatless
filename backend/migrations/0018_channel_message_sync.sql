-- Per-message Slack thread sync: track which external message each TaskMessage corresponds to.
ALTER TABLE task_messages ADD COLUMN IF NOT EXISTS channel_external_id TEXT;

-- Unique partial index enables idempotent sync — re-processing the same thread won't create duplicates.
CREATE UNIQUE INDEX IF NOT EXISTS task_messages_channel_ext_id_idx
  ON task_messages (task_id, channel_external_id)
  WHERE channel_external_id IS NOT NULL;
