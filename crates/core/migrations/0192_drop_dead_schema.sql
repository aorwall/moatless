-- =========================================================================
-- Drop schema nothing reads or writes
-- =========================================================================
-- The preview server columns lost their last reader in #999. The
-- `github_app*` tables were superseded by `0117_adapter_apps.sql`'s
-- `adapter_app*` tables. The retired conversation tables were copied into
-- `conversation_*` by `0178_conversation_read_model.sql`.
--
-- Not reversible. One `DROP TABLE` per table: `arch-rules` reads the live
-- table set from these statements and names only the first of a list.

ALTER TABLE repositories
    DROP COLUMN IF EXISTS servers,
    DROP COLUMN IF EXISTS server_start_commands,
    DROP COLUMN IF EXISTS enable_inspector,
    DROP COLUMN IF EXISTS startup_timeout_seconds,
    DROP COLUMN IF EXISTS auto_reload;
ALTER TABLE workspaces DROP COLUMN IF EXISTS servers;

DROP TABLE IF EXISTS github_app_audit_events;
DROP TABLE IF EXISTS github_app_migrations;
DROP TABLE IF EXISTS github_apps;

DROP TABLE IF EXISTS task_message_contexts;
DROP TABLE IF EXISTS task_message_content_blocks;
DROP TABLE IF EXISTS task_message_usage;
DROP TABLE IF EXISTS task_messages;
DROP TABLE IF EXISTS task_activities;
