-- Canonical integration model hard cutover.
--
-- Replaces the legacy channel config/thread/rule/subscription storage with:
--   provider_connections + task_routes + task_triggers + task_bindings +
--   scheduled_jobs + event_receipts.

BEGIN;

-- ---------------------------------------------------------------------------
-- provider_connections: rename scope_value -> external_account_id.
-- ---------------------------------------------------------------------------

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_name = 'provider_connections'
          AND column_name = 'scope_value'
    ) AND NOT EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_name = 'provider_connections'
          AND column_name = 'external_account_id'
    ) THEN
        ALTER TABLE "provider_connections"
            RENAME COLUMN "scope_value" TO "external_account_id";
    END IF;
END $$;

ALTER TABLE "provider_connections"
    ADD COLUMN IF NOT EXISTS "external_account_id" text;

UPDATE "provider_connections"
SET "external_account_id" = COALESCE("external_account_id", 'default')
WHERE "external_account_id" IS NULL;

ALTER TABLE "provider_connections"
    ALTER COLUMN "external_account_id" SET NOT NULL;

DROP INDEX IF EXISTS "provider_connections_provider_scope_idx";
DROP INDEX IF EXISTS "provider_connections_provider_external_account_idx";
CREATE UNIQUE INDEX IF NOT EXISTS "provider_connections_provider_external_account_idx"
    ON "provider_connections" ("provider", "external_account_id");

-- ---------------------------------------------------------------------------
-- Canonical schema.
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS "task_routes" (
    "id" text PRIMARY KEY NOT NULL,
    "repository_id" text NOT NULL,
    "routing_mode" text NOT NULL DEFAULT 'thread_based',
    "ongoing_task_id" text,
    "prompt_template" text,
    "task_name" text,
    "tag_ids" text[] NOT NULL DEFAULT '{}'::text[],
    "run_as_user_id" text,
    "agent_type" text NOT NULL DEFAULT 'claude-code',
    "scope" text NOT NULL DEFAULT 'user',
    "team_id" text REFERENCES "teams"("id"),
    "created_by" text NOT NULL,
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "updated_at" timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS "task_routes_repository_idx"
    ON "task_routes" ("repository_id");
CREATE INDEX IF NOT EXISTS "task_routes_team_idx"
    ON "task_routes" ("team_id");

CREATE TABLE IF NOT EXISTS "task_triggers" (
    "id" text PRIMARY KEY NOT NULL,
    "provider" text NOT NULL,
    "connection_id" text NOT NULL REFERENCES "provider_connections"("id") ON DELETE CASCADE,
    "source_matcher" text NOT NULL,
    "source_name" text,
    "event_filter" jsonb NOT NULL DEFAULT '[]'::jsonb,
    "trigger_config" jsonb,
    "route_id" text NOT NULL REFERENCES "task_routes"("id") ON DELETE CASCADE,
    "allow_direct_send" boolean NOT NULL DEFAULT false,
    "close_task_on_close" boolean NOT NULL DEFAULT true,
    "notify_on_success" boolean NOT NULL DEFAULT false,
    "active" boolean NOT NULL DEFAULT true,
    "scope" text NOT NULL DEFAULT 'user',
    "team_id" text REFERENCES "teams"("id"),
    "created_by" text NOT NULL,
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "updated_at" timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS "task_triggers_provider_source_idx"
    ON "task_triggers" ("provider", "source_matcher");
CREATE INDEX IF NOT EXISTS "task_triggers_connection_idx"
    ON "task_triggers" ("connection_id");
CREATE INDEX IF NOT EXISTS "task_triggers_route_idx"
    ON "task_triggers" ("route_id");

CREATE TABLE IF NOT EXISTS "task_bindings" (
    "id" text PRIMARY KEY NOT NULL,
    "task_id" text NOT NULL,
    "provider" text NOT NULL,
    "connection_id" text NOT NULL REFERENCES "provider_connections"("id") ON DELETE CASCADE,
    "trigger_id" text REFERENCES "task_triggers"("id") ON DELETE SET NULL,
    "subject_kind" text NOT NULL,
    "subject_key" text NOT NULL,
    "destination_config" jsonb NOT NULL DEFAULT '{}'::jsonb,
    "metadata" jsonb NOT NULL DEFAULT '{}'::jsonb,
    "last_external_event_id" text,
    "origin" text NOT NULL DEFAULT 'ingress',
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "last_event_at" timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS "task_bindings_task_subject_idx"
    ON "task_bindings" ("task_id", "provider", "subject_kind", "subject_key");
CREATE INDEX IF NOT EXISTS "task_bindings_subject_idx"
    ON "task_bindings" ("provider", "subject_kind", "subject_key");
CREATE INDEX IF NOT EXISTS "task_bindings_task_idx"
    ON "task_bindings" ("task_id");

CREATE TABLE IF NOT EXISTS "scheduled_jobs" (
    "id" text PRIMARY KEY NOT NULL,
    "route_id" text NOT NULL REFERENCES "task_routes"("id") ON DELETE CASCADE,
    "schedule_expr" text NOT NULL,
    "timezone" text NOT NULL DEFAULT 'UTC',
    "message_template" text NOT NULL,
    "run_as_user_id" text,
    "active" boolean NOT NULL DEFAULT true,
    "last_fired_at" timestamptz,
    "next_fire_at" timestamptz NOT NULL,
    "locked_by" text,
    "locked_until" timestamptz,
    "created_by" text NOT NULL,
    "scope" text NOT NULL DEFAULT 'user',
    "team_id" text REFERENCES "teams"("id"),
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "updated_at" timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS "scheduled_jobs_due_idx"
    ON "scheduled_jobs" ("active", "next_fire_at");
CREATE INDEX IF NOT EXISTS "scheduled_jobs_route_idx"
    ON "scheduled_jobs" ("route_id");

CREATE TABLE IF NOT EXISTS "event_receipts" (
    "provider" text NOT NULL,
    "external_event_id" text NOT NULL,
    "subject_kind" text NOT NULL,
    "subject_key" text NOT NULL,
    "task_id" text NOT NULL,
    "first_seen_at" timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY ("provider", "external_event_id", "task_id")
);

CREATE INDEX IF NOT EXISTS "event_receipts_subject_idx"
    ON "event_receipts" ("provider", "subject_kind", "subject_key");

-- ---------------------------------------------------------------------------
-- Backfill rules -> routes + triggers.
-- ---------------------------------------------------------------------------

INSERT INTO "task_routes" (
    "id", "repository_id", "routing_mode", "ongoing_task_id",
    "prompt_template", "task_name", "tag_ids", "run_as_user_id",
    "agent_type", "scope", "team_id", "created_by", "created_at", "updated_at"
)
SELECT
    'route_' || ir.id,
    ir.repository_id,
    ir.routing_mode,
    ir.ongoing_task_id,
    ir.prompt_template,
    ir.task_name,
    COALESCE(ir.tag_ids, '{}'::text[]),
    ir.run_as_user_id,
    ir.agent_type,
    ir.scope,
    ir.team_id,
    ir.created_by,
    ir.created_at,
    ir.updated_at
FROM "ingress_rules" ir
ON CONFLICT ("id") DO NOTHING;

INSERT INTO "task_triggers" (
    "id", "provider", "connection_id", "source_matcher", "source_name",
    "event_filter", "trigger_config", "route_id",
    "allow_direct_send", "close_task_on_close", "notify_on_success",
    "active", "scope", "team_id", "created_by", "created_at", "updated_at"
)
SELECT
    ir.id,
    ir.provider,
    ir.connection_id,
    ir.scope_matcher,
    ir.source_name,
    COALESCE(ir.event_filter, '[]'::jsonb),
    ir.provider_config,
    'route_' || ir.id,
    COALESCE(ir.allow_direct_send, false),
    COALESCE(ir.close_task_on_close, true),
    COALESCE(ir.notify_on_success, false),
    ir.active,
    ir.scope,
    ir.team_id,
    ir.created_by,
    ir.created_at,
    ir.updated_at
FROM "ingress_rules" ir
ON CONFLICT ("id") DO NOTHING;

-- Backfill remaining legacy channel_configs.
INSERT INTO "provider_connections" (
    "id", "provider", "connection_kind", "external_account_id", "credential_ref",
    "webhook_secret", "transport_config", "created_by", "scope", "team_id",
    "created_at", "updated_at"
)
SELECT DISTINCT ON (cc.provider)
    'pc_' || cc.provider || '_default',
    cc.provider,
    CASE cc.provider
        WHEN 'webhook' THEN 'incoming_webhook'
        ELSE 'legacy_transport'
    END,
    'default',
    NULL,
    cc.provider_config ->> 'webhook_secret',
    jsonb_build_object('source', 'legacy_channel_config'),
    cc.created_by,
    cc.scope,
    cc.team_id,
    cc.created_at,
    cc.updated_at
FROM "channel_configs" cc
WHERE cc.provider <> 'cron'
ON CONFLICT ("provider", "external_account_id") DO NOTHING;

INSERT INTO "task_routes" (
    "id", "repository_id", "routing_mode", "ongoing_task_id",
    "prompt_template", "task_name", "tag_ids", "run_as_user_id",
    "agent_type", "scope", "team_id", "created_by", "created_at", "updated_at"
)
SELECT
    'route_cc_' || cc.id,
    cc.repository_id,
    cc.routing_mode,
    cc.ongoing_task_id,
    cc.prompt_template,
    cc.task_name,
    COALESCE(cc.tag_ids, '{}'::text[]),
    cc.run_as_user_id,
    cc.agent_type,
    cc.scope,
    cc.team_id,
    cc.created_by,
    cc.created_at,
    cc.updated_at
FROM "channel_configs" cc
ON CONFLICT ("id") DO NOTHING;

INSERT INTO "task_triggers" (
    "id", "provider", "connection_id", "source_matcher", "source_name",
    "event_filter", "trigger_config", "route_id",
    "allow_direct_send", "close_task_on_close", "notify_on_success",
    "active", "scope", "team_id", "created_by", "created_at", "updated_at"
)
SELECT
    'tt_cc_' || cc.id,
    cc.provider,
    pc.id,
    cc.source_id,
    cc.source_name,
    COALESCE(cc.provider_config -> 'events_filter', '[]'::jsonb),
    cc.provider_config,
    'route_cc_' || cc.id,
    cc.allow_direct_send,
    COALESCE((cc.provider_config ->> 'close_task_on_close')::boolean, true),
    COALESCE((cc.provider_config ->> 'notify_on_success')::boolean, false),
    cc.active,
    cc.scope,
    cc.team_id,
    cc.created_by,
    cc.created_at,
    cc.updated_at
FROM "channel_configs" cc
LEFT JOIN "provider_connections" pc
  ON pc.provider = cc.provider
 AND pc.external_account_id = 'default'
WHERE cc.provider <> 'cron'
ON CONFLICT ("id") DO NOTHING;

INSERT INTO "scheduled_jobs" (
    "id", "route_id", "schedule_expr", "timezone",
    "message_template", "run_as_user_id", "active", "last_fired_at",
    "next_fire_at", "created_by", "scope", "team_id", "created_at", "updated_at"
)
SELECT
    'sj_cc_' || cc.id,
    'route_cc_' || cc.id,
    cc.provider_config ->> 'schedule',
    COALESCE(cc.provider_config ->> 'timezone', 'UTC'),
    COALESCE(cc.provider_config ->> 'message', 'Scheduled task trigger'),
    COALESCE(cc.run_as_user_id, cc.provider_config ->> 'user_id'),
    cc.active,
    NULL,
    now(),
    cc.created_by,
    cc.scope,
    cc.team_id,
    cc.created_at,
    cc.updated_at
FROM "channel_configs" cc
WHERE cc.provider = 'cron'
  AND cc.provider_config ? 'schedule'
ON CONFLICT ("id") DO NOTHING;

-- Backfill existing subscriptions/bindings.
INSERT INTO "task_bindings" (
    "id", "task_id", "provider", "connection_id", "trigger_id",
    "subject_kind", "subject_key", "destination_config",
    "metadata", "last_external_event_id", "origin", "created_at", "last_event_at"
)
SELECT
    ts.id,
    ts.task_id,
    ts.provider,
    ts.connection_id,
    ts.ingress_rule_id,
    ts.subject_kind,
    ts.subject_key,
    COALESCE(ts.reply_target, '{}'::jsonb),
    COALESCE(ts.metadata, '{}'::jsonb),
    ts.last_channel_external_id,
    ts.created_reason,
    ts.created_at,
    ts.last_event_at
FROM "task_subscriptions" ts
ON CONFLICT ("task_id", "provider", "subject_kind", "subject_key") DO NOTHING;

INSERT INTO "task_bindings" (
    "id", "task_id", "provider", "connection_id", "trigger_id",
    "subject_kind", "subject_key", "destination_config",
    "metadata", "last_external_event_id", "origin", "created_at", "last_event_at"
)
SELECT
    'tb_ct_' || ct.id::text,
    ct.task_id,
    cc.provider,
    pc.id,
    CASE WHEN cc.provider <> 'cron' THEN 'tt_cc_' || cc.id ELSE NULL END,
    CASE cc.provider
        WHEN 'slack' THEN 'slack_thread'
        WHEN 'telegram' THEN 'telegram_thread'
        WHEN 'github_pr' THEN 'github_pr'
        WHEN 'webhook' THEN 'webhook_thread'
        ELSE 'external_thread'
    END,
    CASE
        WHEN cc.provider = 'github_pr'
            THEN (ct.properties ->> 'owner') || '/' ||
                 (ct.properties ->> 'repo') || '#' ||
                 (ct.properties ->> 'pr_number')
        ELSE cc.source_id || ':' || ct.external_thread_id
    END,
    COALESCE(ct.properties, '{}'::jsonb),
    COALESCE(ct.properties, '{}'::jsonb),
    ct.properties ->> 'last_channel_external_id',
    'connected_thread',
    ct.created_at,
    ct.last_message_at
FROM "channel_threads" ct
JOIN "channel_configs" cc ON cc.id = ct.config_id
LEFT JOIN "provider_connections" pc
  ON pc.provider = cc.provider
 AND pc.external_account_id = 'default'
-- Skip rows that would violate NOT NULL on task_bindings.connection_id or
-- task_bindings.subject_key. A missing 'default' provider_connections row
-- (tenant uses a non-'default' external_account_id) or a partial github_pr
-- properties payload would otherwise abort the whole migration. Skipped rows
-- will be reconciled by the runtime on the next inbound event.
WHERE cc.provider <> 'cron'
  AND pc.id IS NOT NULL
  AND (
      cc.provider <> 'github_pr'
      OR (ct.properties ? 'owner'
          AND ct.properties ? 'repo'
          AND ct.properties ? 'pr_number')
  )
ON CONFLICT ("task_id", "provider", "subject_kind", "subject_key") DO NOTHING;

-- Telegram pairing requests now point at task_triggers.id; keep the legacy
-- column name because this table is provider-specific and not part of the
-- public API cutover.
ALTER TABLE "channel_pairing_requests"
    DROP CONSTRAINT IF EXISTS "channel_pairing_requests_config_id_fkey";

-- ---------------------------------------------------------------------------
-- Drop legacy storage.
-- ---------------------------------------------------------------------------

DROP TABLE IF EXISTS "task_subscriptions" CASCADE;
DROP TABLE IF EXISTS "ingress_rules" CASCADE;
DROP TABLE IF EXISTS "channel_threads" CASCADE;
DROP TABLE IF EXISTS "channel_configs" CASCADE;

COMMIT;
