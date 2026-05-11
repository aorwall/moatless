-- Rename message metadata columns away from channel terminology.

ALTER TABLE task_messages
    RENAME COLUMN channel_external_id TO external_event_id;

ALTER TABLE task_messages
    RENAME COLUMN channel_provider TO external_adapter;

ALTER TABLE task_messages
    RENAME COLUMN channel_source_name TO external_source_name;

ALTER INDEX IF EXISTS task_messages_channel_ext_id_idx
    RENAME TO task_messages_external_event_id_idx;
