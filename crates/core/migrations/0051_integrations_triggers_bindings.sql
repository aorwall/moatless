-- Refactor channels into integration connections, unified triggers, adapter
-- event/schedule trigger sources, and trigger receipts.

BEGIN;

CREATE TABLE IF NOT EXISTS "integration_connections" (
    "id" text PRIMARY KEY NOT NULL,
    "adapter_kind" text NOT NULL,
    "connection_kind" text NOT NULL,
    "external_account_id" text NOT NULL,
    "credential_ref" text,
    "webhook_secret" text,
    "transport_config" jsonb NOT NULL DEFAULT '{}'::jsonb,
    "created_by" text NOT NULL,
    "scope" text NOT NULL DEFAULT 'user',
    "team_id" text REFERENCES "teams"("id"),
    "owner_user_id" text,
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "updated_at" timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS "integration_connections_adapter_account_idx"
    ON "integration_connections" ("adapter_kind", "external_account_id");
CREATE INDEX IF NOT EXISTS "integration_connections_adapter_idx"
    ON "integration_connections" ("adapter_kind");

INSERT INTO "integration_connections" (
    "id", "adapter_kind", "connection_kind", "external_account_id",
    "credential_ref", "webhook_secret", "transport_config", "created_by",
    "scope", "team_id", "owner_user_id", "created_at", "updated_at"
)
SELECT
    "id", "provider", "connection_kind", "external_account_id",
    "credential_ref", "webhook_secret", "transport_config", "created_by",
    "scope", "team_id", "owner_user_id", "created_at", "updated_at"
FROM "provider_connections"
ON CONFLICT ("id") DO NOTHING;

CREATE TABLE IF NOT EXISTS "triggers" (
    "id" text PRIMARY KEY NOT NULL,
    "name" text,
    "kind" text NOT NULL,
    "active" boolean NOT NULL DEFAULT true,
    "scope" text NOT NULL DEFAULT 'user',
    "team_id" text REFERENCES "teams"("id"),
    "owner_user_id" text,
    "created_by" text NOT NULL,
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "updated_at" timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS "trigger_rules" (
    "trigger_id" text PRIMARY KEY NOT NULL REFERENCES "triggers"("id") ON DELETE CASCADE,
    "repository_id" text NOT NULL,
    "routing_mode" text NOT NULL DEFAULT 'by_subject',
    "ongoing_task_id" text,
    "prompt" text,
    "task_name" text,
    "tag_ids" text[] NOT NULL DEFAULT '{}'::text[],
    "run_as_user_id" text,
    "agent_type" text NOT NULL DEFAULT 'claude-code'
);

CREATE INDEX IF NOT EXISTS "triggers_kind_idx" ON "triggers" ("kind");
CREATE INDEX IF NOT EXISTS "triggers_team_idx" ON "triggers" ("team_id");
CREATE INDEX IF NOT EXISTS "trigger_rules_repository_idx" ON "trigger_rules" ("repository_id");

CREATE TABLE IF NOT EXISTS "adapter_event_trigger_sources" (
    "trigger_id" text PRIMARY KEY NOT NULL REFERENCES "triggers"("id") ON DELETE CASCADE,
    "adapter_kind" text NOT NULL,
    "connection_id" text NOT NULL REFERENCES "integration_connections"("id") ON DELETE CASCADE,
    "source_matcher" text NOT NULL,
    "source_name" text,
    "event_filter" jsonb NOT NULL DEFAULT '[]'::jsonb,
    "adapter_config" jsonb,
    "allow_direct_send" boolean NOT NULL DEFAULT false,
    "close_task_on_close" boolean NOT NULL DEFAULT true,
    "notify_on_success" boolean NOT NULL DEFAULT false
);

CREATE INDEX IF NOT EXISTS "adapter_event_trigger_sources_adapter_source_idx"
    ON "adapter_event_trigger_sources" ("adapter_kind", "source_matcher");
CREATE INDEX IF NOT EXISTS "adapter_event_trigger_sources_connection_idx"
    ON "adapter_event_trigger_sources" ("connection_id");

CREATE TABLE IF NOT EXISTS "schedule_trigger_sources" (
    "trigger_id" text PRIMARY KEY NOT NULL REFERENCES "triggers"("id") ON DELETE CASCADE,
    "cron_expression" text NOT NULL,
    "timezone" text NOT NULL DEFAULT 'UTC',
    "message_template" text NOT NULL,
    "last_fired_at" timestamptz,
    "next_fire_at" timestamptz NOT NULL,
    "locked_by" text,
    "locked_until" timestamptz
);

CREATE INDEX IF NOT EXISTS "schedule_trigger_sources_due_idx"
    ON "schedule_trigger_sources" ("next_fire_at");

INSERT INTO "triggers" (
    "id", "name", "kind", "active", "scope", "team_id", "owner_user_id",
    "created_by", "created_at", "updated_at"
)
SELECT
    tt."id",
    tt."source_name",
    'adapter_event',
    tt."active",
    tt."scope",
    tt."team_id",
    tt."owner_user_id",
    tt."created_by",
    tt."created_at",
    tt."updated_at"
FROM "task_triggers" tt
ON CONFLICT ("id") DO NOTHING;

INSERT INTO "trigger_rules" (
    "trigger_id", "repository_id", "routing_mode", "ongoing_task_id", "prompt",
    "task_name", "tag_ids", "run_as_user_id", "agent_type"
)
SELECT
    tt."id",
    tr."repository_id",
    tr."routing_mode",
    tr."ongoing_task_id",
    tr."prompt_template",
    tr."task_name",
    tr."tag_ids",
    tr."run_as_user_id",
    tr."agent_type"
FROM "task_triggers" tt
JOIN "task_routes" tr ON tr."id" = tt."route_id"
ON CONFLICT ("trigger_id") DO NOTHING;

INSERT INTO "adapter_event_trigger_sources" (
    "trigger_id", "adapter_kind", "connection_id", "source_matcher",
    "source_name", "event_filter", "adapter_config", "allow_direct_send",
    "close_task_on_close", "notify_on_success"
)
SELECT
    tt."id",
    tt."provider",
    tt."connection_id",
    tt."source_matcher",
    tt."source_name",
    COALESCE(tt."event_filter", '[]'::jsonb),
    tt."trigger_config",
    tt."allow_direct_send",
    tt."close_task_on_close",
    tt."notify_on_success"
FROM "task_triggers" tt
ON CONFLICT ("trigger_id") DO NOTHING;

INSERT INTO "triggers" (
    "id", "name", "kind", "active", "scope", "team_id", "owner_user_id",
    "created_by", "created_at", "updated_at"
)
SELECT
    sj."id",
    NULL,
    'schedule',
    sj."active",
    sj."scope",
    sj."team_id",
    sj."owner_user_id",
    sj."created_by",
    sj."created_at",
    sj."updated_at"
FROM "scheduled_jobs" sj
ON CONFLICT ("id") DO NOTHING;

INSERT INTO "trigger_rules" (
    "trigger_id", "repository_id", "routing_mode", "ongoing_task_id", "prompt",
    "task_name", "tag_ids", "run_as_user_id", "agent_type"
)
SELECT
    sj."id",
    tr."repository_id",
    tr."routing_mode",
    tr."ongoing_task_id",
    tr."prompt_template",
    tr."task_name",
    tr."tag_ids",
    COALESCE(sj."run_as_user_id", tr."run_as_user_id"),
    tr."agent_type"
FROM "scheduled_jobs" sj
JOIN "task_routes" tr ON tr."id" = sj."route_id"
ON CONFLICT ("trigger_id") DO NOTHING;

INSERT INTO "schedule_trigger_sources" (
    "trigger_id", "cron_expression", "timezone", "message_template",
    "last_fired_at", "next_fire_at", "locked_by", "locked_until"
)
SELECT
    "id",
    "schedule_expr",
    "timezone",
    "message_template",
    "last_fired_at",
    "next_fire_at",
    "locked_by",
    "locked_until"
FROM "scheduled_jobs"
ON CONFLICT ("trigger_id") DO NOTHING;

-- Preserve orphan routes as inactive manual triggers so existing data is not lost.
INSERT INTO "triggers" (
    "id", "name", "kind", "active", "scope", "team_id", "owner_user_id",
    "created_by", "created_at", "updated_at"
)
SELECT
    tr."id",
    tr."task_name",
    'manual',
    false,
    tr."scope",
    tr."team_id",
    tr."owner_user_id",
    tr."created_by",
    tr."created_at",
    tr."updated_at"
FROM "task_routes" tr
WHERE NOT EXISTS (SELECT 1 FROM "task_triggers" tt WHERE tt."route_id" = tr."id")
  AND NOT EXISTS (SELECT 1 FROM "scheduled_jobs" sj WHERE sj."route_id" = tr."id")
ON CONFLICT ("id") DO NOTHING;

INSERT INTO "trigger_rules" (
    "trigger_id", "repository_id", "routing_mode", "ongoing_task_id", "prompt",
    "task_name", "tag_ids", "run_as_user_id", "agent_type"
)
SELECT
    tr."id",
    tr."repository_id",
    tr."routing_mode",
    tr."ongoing_task_id",
    tr."prompt_template",
    tr."task_name",
    tr."tag_ids",
    tr."run_as_user_id",
    tr."agent_type"
FROM "task_routes" tr
WHERE NOT EXISTS (SELECT 1 FROM "task_triggers" tt WHERE tt."route_id" = tr."id")
  AND NOT EXISTS (SELECT 1 FROM "scheduled_jobs" sj WHERE sj."route_id" = tr."id")
ON CONFLICT ("trigger_id") DO NOTHING;

CREATE TABLE IF NOT EXISTS "trigger_receipts" (
    "adapter_kind" text NOT NULL,
    "external_event_id" text NOT NULL,
    "subject_kind" text NOT NULL,
    "subject_key" text NOT NULL,
    "task_id" text NOT NULL,
    "first_seen_at" timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY ("adapter_kind", "external_event_id", "task_id")
);

CREATE INDEX IF NOT EXISTS "trigger_receipts_subject_idx"
    ON "trigger_receipts" ("adapter_kind", "subject_kind", "subject_key");

INSERT INTO "trigger_receipts" (
    "adapter_kind", "external_event_id", "subject_kind", "subject_key",
    "task_id", "first_seen_at"
)
SELECT
    "provider", "external_event_id", "subject_kind", "subject_key",
    "task_id", "first_seen_at"
FROM "event_receipts"
ON CONFLICT ("adapter_kind", "external_event_id", "task_id") DO NOTHING;

ALTER TABLE "task_bindings" DROP CONSTRAINT IF EXISTS "task_bindings_connection_id_fkey";
ALTER TABLE "task_bindings" DROP CONSTRAINT IF EXISTS "task_bindings_trigger_id_fkey";

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'task_bindings' AND column_name = 'provider'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'task_bindings' AND column_name = 'adapter_kind'
    ) THEN
        ALTER TABLE "task_bindings" RENAME COLUMN "provider" TO "adapter_kind";
    END IF;
END $$;

ALTER TABLE "task_bindings"
    ADD CONSTRAINT "task_bindings_connection_id_fkey"
    FOREIGN KEY ("connection_id") REFERENCES "integration_connections"("id") ON DELETE CASCADE;
ALTER TABLE "task_bindings"
    ADD CONSTRAINT "task_bindings_trigger_id_fkey"
    FOREIGN KEY ("trigger_id") REFERENCES "triggers"("id") ON DELETE SET NULL;

DROP INDEX IF EXISTS "task_bindings_task_subject_idx";
DROP INDEX IF EXISTS "task_bindings_subject_idx";
CREATE UNIQUE INDEX IF NOT EXISTS "task_bindings_task_subject_idx"
    ON "task_bindings" ("task_id", "adapter_kind", "subject_kind", "subject_key");
CREATE INDEX IF NOT EXISTS "task_bindings_subject_idx"
    ON "task_bindings" ("adapter_kind", "subject_kind", "subject_key");

DROP TABLE IF EXISTS "event_receipts" CASCADE;
DROP TABLE IF EXISTS "scheduled_jobs" CASCADE;
DROP TABLE IF EXISTS "task_triggers" CASCADE;
DROP TABLE IF EXISTS "task_routes" CASCADE;
DROP TABLE IF EXISTS "provider_connections" CASCADE;

COMMIT;
