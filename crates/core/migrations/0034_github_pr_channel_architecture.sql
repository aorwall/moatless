-- GitHub PR channel: org-webhook + ingress rules + task subscriptions.
--
-- Rebuilds the GitHub side of the channel architecture around three new tables:
--   * provider_connections — one row per transport/auth (e.g. one org webhook)
--   * ingress_rules        — one row per (connection, scope_matcher) routing rule
--   * task_subscriptions   — one row per task ↔ external subject (e.g. PR) link
--
-- This migration is one-shot. It is intended to be applied **manually** by the
-- operator first, before the new backend image is deployed. See
-- docs/runbooks/github-channel-org-webhook.md for the full cutover.
--
-- Operator workflow (TL;DR):
--   1) Generate a webhook secret and register the org webhook on Frank-Penny.
--   2) Run this migration via psql, injecting the secret as a server-side
--      configuration parameter:
--           kubectl -n moatless run pg-migrate --rm -it \
--             --image=postgres:17-alpine --restart=Never -- \
--             psql "<DATABASE_URL>" \
--               -v gh_org_webhook_secret="$(cat /tmp/gh-org-webhook-secret)" \
--               -c "SET app.gh_org_webhook_secret = :'gh_org_webhook_secret';" \
--               -f 0034_github_pr_channel_architecture.sql
--      The `RAISE NOTICE` output must show
--      `provider_connections=1, ingress_rules=6, task_subscriptions=N`.
--   3) Deploy the new backend image — sqlx will see this migration as already
--      applied (idempotent CREATE TABLE; backfill DO block skips itself when
--      `app.gh_org_webhook_secret` is unset) and continue past it.
--
-- The DO block is intentionally a no-op when sqlx::migrate runs it at backend
-- startup (no GUC set) so the schema creation is the only effect there.

BEGIN;

-- ---------------------------------------------------------------------------
-- Schema
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS "provider_connections" (
    "id" text PRIMARY KEY NOT NULL,
    "provider" text NOT NULL,
    "connection_kind" text NOT NULL,
    "scope_value" text NOT NULL,
    "credential_ref" text,
    "webhook_secret" text,
    "transport_config" jsonb NOT NULL DEFAULT '{}'::jsonb,
    "created_by" text NOT NULL,
    "scope" text NOT NULL DEFAULT 'user',
    "team_id" text REFERENCES "teams"("id"),
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "updated_at" timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS "provider_connections_provider_scope_idx"
    ON "provider_connections" ("provider", "scope_value");
CREATE INDEX IF NOT EXISTS "provider_connections_provider_idx"
    ON "provider_connections" ("provider");

CREATE TABLE IF NOT EXISTS "ingress_rules" (
    "id" text PRIMARY KEY NOT NULL,
    "provider" text NOT NULL,
    "connection_id" text NOT NULL REFERENCES "provider_connections"("id") ON DELETE CASCADE,
    "scope_matcher" text NOT NULL,
    "event_filter" jsonb NOT NULL DEFAULT '[]'::jsonb,
    "repository_id" text NOT NULL,
    "prompt_template" text,
    "task_name" text,
    "tag_ids" text[] NOT NULL DEFAULT '{}'::text[],
    "run_as_user_id" text,
    "agent_type" text NOT NULL DEFAULT 'claude-code',
    "close_task_on_close" boolean NOT NULL DEFAULT true,
    "notify_on_success" boolean NOT NULL DEFAULT false,
    "active" boolean NOT NULL DEFAULT true,
    "created_by" text NOT NULL,
    "scope" text NOT NULL DEFAULT 'user',
    "team_id" text REFERENCES "teams"("id"),
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "updated_at" timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS "ingress_rules_provider_scope_idx"
    ON "ingress_rules" ("provider", "scope_matcher");
CREATE INDEX IF NOT EXISTS "ingress_rules_connection_idx"
    ON "ingress_rules" ("connection_id");

CREATE TABLE IF NOT EXISTS "task_subscriptions" (
    "id" text PRIMARY KEY NOT NULL,
    "task_id" text NOT NULL,
    "provider" text NOT NULL,
    "subject_kind" text NOT NULL,
    "subject_key" text NOT NULL,
    "connection_id" text NOT NULL REFERENCES "provider_connections"("id") ON DELETE CASCADE,
    "ingress_rule_id" text REFERENCES "ingress_rules"("id") ON DELETE SET NULL,
    "reply_target" jsonb NOT NULL DEFAULT '{}'::jsonb,
    "metadata" jsonb NOT NULL DEFAULT '{}'::jsonb,
    "last_channel_external_id" text,
    "created_reason" text NOT NULL,
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "last_event_at" timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS "task_subscriptions_task_subject_idx"
    ON "task_subscriptions" ("task_id", "provider", "subject_kind", "subject_key");
CREATE INDEX IF NOT EXISTS "task_subscriptions_subject_idx"
    ON "task_subscriptions" ("provider", "subject_kind", "subject_key");
CREATE INDEX IF NOT EXISTS "task_subscriptions_task_idx"
    ON "task_subscriptions" ("task_id");

-- ---------------------------------------------------------------------------
-- Backfill — only runs when the operator has set the
-- `app.gh_org_webhook_secret` GUC for the current session. sqlx auto-migrate
-- will not have it set, so the block is a no-op there.
-- ---------------------------------------------------------------------------

DO $migrate$
DECLARE
    secret           text;
    owner_user_id    text;
    org_connection_id text := 'pc_gh_org_frank_penny';
    skipped_threads  integer;
    conn_count       integer;
    rule_count       integer;
    sub_count        integer;
    leftover_configs integer;
    leftover_threads integer;
BEGIN
    secret := current_setting('app.gh_org_webhook_secret', true);
    IF secret IS NULL OR length(secret) = 0 THEN
        RAISE NOTICE
            'Skipping GitHub org-webhook backfill (app.gh_org_webhook_secret not set). '
            'This is expected when sqlx::migrate runs the migration at backend startup; '
            'apply the migration manually via psql with -c "SET app.gh_org_webhook_secret = ..." '
            'to perform the org-webhook backfill.';
        RETURN;
    END IF;

    -- Resolve the user that owns the most existing github_pr configs.
    SELECT cc.created_by INTO owner_user_id
    FROM "channel_configs" cc
    WHERE cc.provider = 'github_pr'
    GROUP BY cc.created_by
    ORDER BY count(*) DESC, min(cc.created_at) ASC
    LIMIT 1;

    IF owner_user_id IS NULL THEN
        RAISE NOTICE 'No github_pr channel_configs found; backfill is a no-op.';
        RETURN;
    END IF;

    -- 1. Org webhook connection.
    INSERT INTO "provider_connections" (
        "id", "provider", "connection_kind", "scope_value",
        "credential_ref", "webhook_secret", "transport_config",
        "created_by", "scope", "team_id", "created_at", "updated_at"
    )
    VALUES (
        org_connection_id,
        'github',
        'org_webhook',
        'Frank-Penny',
        NULL,
        secret,
        '{}'::jsonb,
        owner_user_id,
        'user',
        NULL,
        now(),
        now()
    )
    ON CONFLICT ("provider", "scope_value") DO UPDATE
    SET webhook_secret = EXCLUDED.webhook_secret,
        updated_at     = now();

    -- 2. One ingress rule per existing github_pr config.
    INSERT INTO "ingress_rules" (
        "id", "provider", "connection_id", "scope_matcher", "event_filter",
        "repository_id", "prompt_template", "task_name", "tag_ids",
        "run_as_user_id", "agent_type", "close_task_on_close",
        "notify_on_success", "active", "created_by", "scope", "team_id",
        "created_at", "updated_at"
    )
    SELECT
        'ir_' || cc.id,
        'github',
        org_connection_id,
        cc.source_id,
        COALESCE(cc.provider_config -> 'events_filter', '[]'::jsonb),
        cc.repository_id,
        cc.prompt_template,
        cc.task_name,
        COALESCE(cc.tag_ids, '{}'::text[]),
        cc.run_as_user_id,
        cc.agent_type,
        COALESCE((cc.provider_config ->> 'close_task_on_close')::boolean, true),
        COALESCE((cc.provider_config ->> 'notify_on_success')::boolean, false),
        cc.active,
        cc.created_by,
        cc.scope,
        cc.team_id,
        cc.created_at,
        now()
    FROM "channel_configs" cc
    WHERE cc.provider = 'github_pr'
    ON CONFLICT ("provider", "scope_matcher") DO NOTHING;

    -- 3. Task subscription per legacy channel_thread (drop rows with bad props).
    SELECT count(*) INTO skipped_threads
    FROM "channel_threads" ct
    JOIN "channel_configs" cc ON cc.id = ct.config_id
    WHERE cc.provider = 'github_pr'
      AND (
            NOT (ct.properties ? 'owner') OR
            NOT (ct.properties ? 'repo')  OR
            NOT (ct.properties ? 'pr_number')
      );
    IF skipped_threads > 0 THEN
        RAISE NOTICE 'Dropping % github_pr channel_threads with missing owner/repo/pr_number', skipped_threads;
    END IF;

    INSERT INTO "task_subscriptions" (
        "id", "task_id", "provider", "subject_kind", "subject_key",
        "connection_id", "ingress_rule_id", "reply_target", "metadata",
        "created_reason", "created_at", "last_event_at"
    )
    SELECT
        'ts_' || ct.id::text,
        ct.task_id,
        'github',
        'github_pr',
        (ct.properties ->> 'owner') || '/' || (ct.properties ->> 'repo') || '#' || (ct.properties ->> 'pr_number'),
        org_connection_id,
        ir.id,
        jsonb_build_object(
            'owner', ct.properties ->> 'owner',
            'repo', ct.properties ->> 'repo',
            'pr_number', (ct.properties ->> 'pr_number')::bigint
        ),
        jsonb_build_object(
            'filter_user_id', ct.properties ->> 'filter_user_id'
        ),
        'connected_thread',
        ct.created_at,
        ct.last_message_at
    FROM "channel_threads" ct
    JOIN "channel_configs" cc ON cc.id = ct.config_id
    LEFT JOIN "ingress_rules" ir
      ON ir.provider = 'github' AND ir.scope_matcher = cc.source_id
    WHERE cc.provider = 'github_pr'
      AND (ct.properties ? 'owner')
      AND (ct.properties ? 'repo')
      AND (ct.properties ? 'pr_number')
    ON CONFLICT ("task_id", "provider", "subject_kind", "subject_key") DO NOTHING;

    -- 4. Drop the legacy github_pr rows.
    DELETE FROM "channel_threads"
    WHERE config_id IN (
        SELECT id FROM "channel_configs" WHERE provider = 'github_pr'
    );
    DELETE FROM "channel_configs" WHERE provider = 'github_pr';

    -- 5. Counts for the deploy log.
    SELECT count(*) INTO conn_count
    FROM "provider_connections" WHERE provider = 'github';
    SELECT count(*) INTO rule_count
    FROM "ingress_rules" WHERE provider = 'github';
    SELECT count(*) INTO sub_count
    FROM "task_subscriptions" WHERE provider = 'github';
    SELECT count(*) INTO leftover_configs
    FROM "channel_configs" WHERE provider = 'github_pr';
    SELECT count(*) INTO leftover_threads
    FROM "channel_threads" ct
    JOIN "channel_configs" cc ON cc.id = ct.config_id
    WHERE cc.provider = 'github_pr';

    RAISE NOTICE
        'GitHub channel migration: provider_connections=%, ingress_rules=%, task_subscriptions=%, leftover_configs=%, leftover_threads=%',
        conn_count, rule_count, sub_count, leftover_configs, leftover_threads;

    IF conn_count <> 1 THEN
        RAISE EXCEPTION 'Expected exactly 1 github provider_connections row, got %', conn_count;
    END IF;
    IF leftover_configs <> 0 OR leftover_threads <> 0 THEN
        RAISE EXCEPTION 'Legacy github_pr rows still present (configs=%, threads=%)',
            leftover_configs, leftover_threads;
    END IF;
END
$migrate$;

COMMIT;
