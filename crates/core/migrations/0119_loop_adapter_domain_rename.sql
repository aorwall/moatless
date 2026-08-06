-- Align the physical schema with the Loop/Subscription/Adapter glossary.
--
-- This intentionally follows the stop-the-world precedent from
-- 0106_adapter_delivery_rename.sql: no compatibility views, aliases, or old
-- route handlers. The database is the canonical vocabulary, and application
-- code changes in the same deploy.

BEGIN;

ALTER TABLE "integration_connections" RENAME TO "adapter_connections";
ALTER TABLE "triggers" RENAME TO "loops";
ALTER TABLE "trigger_rules" RENAME TO "loop_configs";
ALTER TABLE "adapter_event_trigger_sources" RENAME TO "subscriptions";
ALTER TABLE "schedule_trigger_sources" RENAME TO "schedules";
ALTER TABLE "trigger_receipts" RENAME TO "adapter_event_receipts";

ALTER TABLE "loop_configs" RENAME COLUMN "trigger_id" TO "loop_id";
ALTER TABLE "subscriptions" RENAME COLUMN "trigger_id" TO "loop_id";
ALTER TABLE "schedules" RENAME COLUMN "trigger_id" TO "loop_id";
ALTER TABLE "task_bindings" RENAME COLUMN "trigger_id" TO "loop_id";
ALTER TABLE "tasks" RENAME COLUMN "trigger_id" TO "loop_id";
ALTER TABLE "integration_pairing_requests" RENAME COLUMN "trigger_id" TO "loop_id";

ALTER TABLE "adapter_connections" RENAME CONSTRAINT "integration_connections_pkey"
    TO "adapter_connections_pkey";
ALTER TABLE "adapter_connections" RENAME CONSTRAINT "integration_connections_team_id_fkey"
    TO "adapter_connections_team_id_fkey";
ALTER TABLE "adapter_connections" RENAME CONSTRAINT "integration_connections_owner_scope_chk"
    TO "adapter_connections_owner_scope_chk";
ALTER TABLE "loops" RENAME CONSTRAINT "triggers_pkey"
    TO "loops_pkey";
ALTER TABLE "loops" RENAME CONSTRAINT "triggers_team_id_fkey"
    TO "loops_team_id_fkey";
ALTER TABLE "loops" RENAME CONSTRAINT "triggers_owner_scope_chk"
    TO "loops_owner_scope_chk";
ALTER TABLE "loops" RENAME CONSTRAINT "triggers_config_source_check"
    TO "loops_config_source_check";
ALTER TABLE "loops" RENAME CONSTRAINT "triggers_name_not_blank_check"
    TO "loops_name_not_blank_check";
ALTER TABLE "loop_configs" RENAME CONSTRAINT "trigger_rules_pkey"
    TO "loop_configs_pkey";
ALTER TABLE "loop_configs" RENAME CONSTRAINT "trigger_rules_trigger_id_fkey"
    TO "loop_configs_loop_id_fkey";
ALTER TABLE "subscriptions" RENAME CONSTRAINT "adapter_event_trigger_sources_pkey"
    TO "subscriptions_pkey";
ALTER TABLE "subscriptions" RENAME CONSTRAINT "adapter_event_trigger_sources_trigger_id_fkey"
    TO "subscriptions_loop_id_fkey";
ALTER TABLE "subscriptions" RENAME CONSTRAINT "adapter_event_trigger_sources_connection_id_fkey"
    TO "subscriptions_connection_id_fkey";
ALTER TABLE "schedules" RENAME CONSTRAINT "schedule_trigger_sources_pkey"
    TO "schedules_pkey";
ALTER TABLE "schedules" RENAME CONSTRAINT "schedule_trigger_sources_trigger_id_fkey"
    TO "schedules_loop_id_fkey";
ALTER TABLE "adapter_event_receipts" RENAME CONSTRAINT "trigger_receipts_pkey"
    TO "adapter_event_receipts_pkey";

ALTER TABLE "task_bindings" RENAME CONSTRAINT "task_bindings_trigger_id_fkey"
    TO "task_bindings_loop_id_fkey";
ALTER TABLE "tasks" RENAME CONSTRAINT "tasks_trigger_id_fkey"
    TO "tasks_loop_id_fkey";
ALTER TABLE "integration_pairing_requests" RENAME CONSTRAINT "integration_pairing_requests_trigger_id_fkey"
    TO "integration_pairing_requests_loop_id_fkey";

ALTER INDEX IF EXISTS "integration_connections_adapter_account_idx"
    RENAME TO "adapter_connections_adapter_account_idx";
ALTER INDEX IF EXISTS "integration_connections_adapter_idx"
    RENAME TO "adapter_connections_adapter_idx";
ALTER INDEX IF EXISTS "triggers_kind_idx"
    RENAME TO "loops_kind_idx";
ALTER INDEX IF EXISTS "triggers_team_idx"
    RENAME TO "loops_team_idx";
ALTER INDEX IF EXISTS "triggers_git_provenance_idx"
    RENAME TO "loops_git_provenance_idx";
ALTER INDEX IF EXISTS "triggers_source_repository_idx"
    RENAME TO "loops_source_repository_idx";
ALTER INDEX IF EXISTS "trigger_rules_repository_idx"
    RENAME TO "loop_configs_repository_idx";
ALTER INDEX IF EXISTS "adapter_event_trigger_sources_adapter_source_idx"
    RENAME TO "subscriptions_adapter_source_idx";
ALTER INDEX IF EXISTS "adapter_event_trigger_sources_connection_idx"
    RENAME TO "subscriptions_connection_idx";
ALTER INDEX IF EXISTS "schedule_trigger_sources_due_idx"
    RENAME TO "schedules_due_idx";
ALTER INDEX IF EXISTS "trigger_receipts_subject_idx"
    RENAME TO "adapter_event_receipts_subject_idx";
ALTER INDEX IF EXISTS "tasks_trigger_id_idx"
    RENAME TO "tasks_loop_id_idx";
ALTER INDEX IF EXISTS "integration_pairing_requests_pending_user_idx"
    RENAME TO "integration_pairing_requests_pending_user_by_loop_idx";
ALTER INDEX IF EXISTS "integration_pairing_requests_pending_code_idx"
    RENAME TO "integration_pairing_requests_pending_code_by_loop_idx";

-- Durable Adapter delivery payloads are stored as JSONB. Rewrite the few
-- renamed field keys so already-admitted records deserialize under the new
-- Rust names after the deploy.
UPDATE "adapter_delivery"
SET "payload" = jsonb_set("payload" - 'rule_id', '{loop_id}', "payload"->'rule_id')
WHERE "payload" ? 'rule_id';

UPDATE "adapter_delivery"
SET "payload" = jsonb_set("payload" - 'trigger_id', '{loop_id}', "payload"->'trigger_id')
WHERE "payload" ? 'trigger_id';

UPDATE "adapter_delivery"
SET "payload" = jsonb_set(
        "payload" #- '{message,provider}',
        '{message,adapter_kind}',
        "payload"#>'{message,provider}'
    )
WHERE "payload"#>'{message,provider}' IS NOT NULL;

UPDATE "adapter_delivery"
SET "payload" = jsonb_set(
        "payload" #- '{message,provider_context}',
        '{message,adapter_context}',
        "payload"#>'{message,provider_context}'
    )
WHERE "payload"#>'{message,provider_context}' IS NOT NULL;

COMMENT ON TABLE "adapter_connections" IS
    'Adapter connection records: credentials and sender/account metadata for one Adapter.';
COMMENT ON TABLE "loops" IS
    'Standing definitions of recurring agent work.';
COMMENT ON TABLE "loop_configs" IS
    'Shared task-spawn configuration for a Loop.';
COMMENT ON TABLE "subscriptions" IS
    'Adapter-event firing definitions for Loops.';
COMMENT ON TABLE "schedules" IS
    'Schedule firing definitions for Loops.';
COMMENT ON TABLE "adapter_event_receipts" IS
    'Deduplication receipts for Adapter events delivered to Tasks.';

COMMIT;
