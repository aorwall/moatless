-- Slack/Telegram hard cutover to provider_connections + ingress_rules +
-- task_subscriptions. webhook and cron remain on channel_configs.

-- ---------------------------------------------------------------------------
-- Schema: chat-channel fields formerly stored on channel_configs.
-- ---------------------------------------------------------------------------

ALTER TABLE "ingress_rules"
    ADD COLUMN IF NOT EXISTS "source_name" text,
    ADD COLUMN IF NOT EXISTS "routing_mode" text NOT NULL DEFAULT 'thread_based',
    ADD COLUMN IF NOT EXISTS "allow_direct_send" boolean NOT NULL DEFAULT false,
    ADD COLUMN IF NOT EXISTS "provider_config" jsonb,
    ADD COLUMN IF NOT EXISTS "ongoing_task_id" text,
    ADD COLUMN IF NOT EXISTS "response_rule_id" text REFERENCES "ingress_rules"("id") ON DELETE SET NULL;

-- Telegram pairing now stores ingress_rules.id in config_id for compatibility
-- with the existing API field name.
ALTER TABLE "channel_pairing_requests"
    DROP CONSTRAINT IF EXISTS "channel_pairing_requests_config_id_fkey";

-- ---------------------------------------------------------------------------
-- Provider connections: one env-backed transport per provider.
-- ---------------------------------------------------------------------------

INSERT INTO "provider_connections" (
    "id", "provider", "connection_kind", "scope_value", "credential_ref",
    "webhook_secret", "transport_config", "created_by", "scope", "team_id",
    "created_at", "updated_at"
)
SELECT
    'pc_' || provider || '_default',
    provider,
    CASE provider WHEN 'slack' THEN 'events_api' ELSE 'bot_api' END,
    'default',
    provider || ':env',
    NULL,
    '{"source":"env"}'::jsonb,
    COALESCE(
        (SELECT cc.created_by
         FROM "channel_configs" cc
         WHERE cc.provider = p.provider
         GROUP BY cc.created_by
         ORDER BY count(*) DESC, min(cc.created_at) ASC
         LIMIT 1),
        'system'
    ),
    'global',
    NULL,
    now(),
    now()
FROM (VALUES ('slack'), ('telegram')) AS p(provider)
WHERE EXISTS (SELECT 1 FROM "channel_configs" cc WHERE cc.provider = p.provider)
ON CONFLICT ("provider", "scope_value") DO UPDATE
SET connection_kind  = EXCLUDED.connection_kind,
    credential_ref   = EXCLUDED.credential_ref,
    transport_config = EXCLUDED.transport_config,
    updated_at       = now();

-- ---------------------------------------------------------------------------
-- Rules: one per Slack/Telegram config.
-- ---------------------------------------------------------------------------

INSERT INTO "ingress_rules" (
    "id", "provider", "connection_id", "scope_matcher", "source_name",
    "routing_mode", "event_filter", "repository_id", "prompt_template",
    "task_name", "tag_ids", "run_as_user_id", "agent_type",
    "allow_direct_send", "provider_config", "ongoing_task_id",
    "close_task_on_close", "notify_on_success", "active",
    "created_by", "scope", "team_id", "created_at", "updated_at"
)
SELECT
    'ir_' || cc.id,
    cc.provider,
    pc.id,
    cc.source_id,
    cc.source_name,
    cc.routing_mode,
    '[]'::jsonb,
    cc.repository_id,
    cc.prompt_template,
    cc.task_name,
    COALESCE(cc.tag_ids, '{}'::text[]),
    cc.run_as_user_id,
    cc.agent_type,
    cc.allow_direct_send,
    cc.provider_config,
    cc.ongoing_task_id,
    true,
    false,
    cc.active,
    cc.created_by,
    cc.scope,
    cc.team_id,
    cc.created_at,
    now()
FROM "channel_configs" cc
JOIN "provider_connections" pc
  ON pc.provider = cc.provider AND pc.scope_value = 'default'
WHERE cc.provider IN ('slack', 'telegram')
ON CONFLICT ("provider", "scope_matcher") DO UPDATE
SET connection_id     = EXCLUDED.connection_id,
    source_name       = EXCLUDED.source_name,
    routing_mode      = EXCLUDED.routing_mode,
    repository_id     = EXCLUDED.repository_id,
    prompt_template   = EXCLUDED.prompt_template,
    task_name         = EXCLUDED.task_name,
    tag_ids           = EXCLUDED.tag_ids,
    run_as_user_id    = EXCLUDED.run_as_user_id,
    agent_type        = EXCLUDED.agent_type,
    allow_direct_send = EXCLUDED.allow_direct_send,
    provider_config   = EXCLUDED.provider_config,
    ongoing_task_id   = EXCLUDED.ongoing_task_id,
    active            = EXCLUDED.active,
    updated_at        = now();

-- Rewire response targets when both sides moved to rules.
UPDATE "ingress_rules" ir
SET response_rule_id = ('ir_' || cc.response_config_id)
FROM "channel_configs" cc
WHERE ir.id = ('ir_' || cc.id)
  AND cc.provider IN ('slack', 'telegram')
  AND cc.response_config_id IS NOT NULL
  AND EXISTS (
      SELECT 1 FROM "channel_configs" rc
      WHERE rc.id = cc.response_config_id
        AND rc.provider IN ('slack', 'telegram')
  );

-- ---------------------------------------------------------------------------
-- Subscriptions from legacy threads.
-- ---------------------------------------------------------------------------

INSERT INTO "task_subscriptions" (
    "id", "task_id", "provider", "subject_kind", "subject_key",
    "connection_id", "ingress_rule_id", "reply_target", "metadata",
    "last_channel_external_id", "created_reason", "created_at", "last_event_at"
)
SELECT
    'ts_' || ct.id::text,
    ct.task_id,
    'slack',
    'slack_thread',
    COALESCE(cc.provider_config ->> 'team_id', 'default') || ':' ||
        COALESCE(ct.properties ->> 'channel', cc.source_id) || ':' ||
        COALESCE(ct.properties ->> 'thread_ts', ct.external_thread_id),
    pc.id,
    ir.id,
    jsonb_build_object(
        'channel', COALESCE(ct.properties ->> 'channel', cc.source_id),
        'thread_ts', COALESCE(ct.properties ->> 'thread_ts', ct.external_thread_id),
        'ts', COALESCE(ct.properties ->> 'ts', ct.external_thread_id)
    ),
    jsonb_build_object('filter_user_id', ct.properties -> 'filter_user_id'),
    ct.properties ->> 'last_channel_external_id',
    'connected_thread',
    ct.created_at,
    ct.last_message_at
FROM "channel_threads" ct
JOIN "channel_configs" cc ON cc.id = ct.config_id
JOIN "provider_connections" pc ON pc.provider = 'slack' AND pc.scope_value = 'default'
LEFT JOIN "ingress_rules" ir ON ir.provider = 'slack' AND ir.scope_matcher = cc.source_id
WHERE cc.provider = 'slack'
ON CONFLICT ("task_id", "provider", "subject_kind", "subject_key") DO NOTHING;

INSERT INTO "task_subscriptions" (
    "id", "task_id", "provider", "subject_kind", "subject_key",
    "connection_id", "ingress_rule_id", "reply_target", "metadata",
    "last_channel_external_id", "created_reason", "created_at", "last_event_at"
)
SELECT
    'ts_' || ct.id::text,
    ct.task_id,
    'telegram',
    'telegram_thread',
    COALESCE(ct.properties ->> 'chat_id', cc.source_id) ||
        CASE
            WHEN ct.properties ? 'forum_topic_id' AND ct.properties ->> 'forum_topic_id' IS NOT NULL
                THEN ':topic:' || (ct.properties ->> 'forum_topic_id')
            ELSE ':thread:' || ct.external_thread_id
        END,
    pc.id,
    ir.id,
    jsonb_build_object(
        'chat_id', COALESCE(ct.properties ->> 'chat_id', cc.source_id),
        'message_id', COALESCE((ct.properties ->> 'message_id')::bigint, NULL),
        'forum_topic_id', COALESCE((ct.properties ->> 'forum_topic_id')::bigint, NULL)
    ),
    jsonb_build_object('filter_user_id', ct.properties -> 'filter_user_id'),
    ct.properties ->> 'last_channel_external_id',
    'connected_thread',
    ct.created_at,
    ct.last_message_at
FROM "channel_threads" ct
JOIN "channel_configs" cc ON cc.id = ct.config_id
JOIN "provider_connections" pc ON pc.provider = 'telegram' AND pc.scope_value = 'default'
LEFT JOIN "ingress_rules" ir ON ir.provider = 'telegram' AND ir.scope_matcher = cc.source_id
WHERE cc.provider = 'telegram'
ON CONFLICT ("task_id", "provider", "subject_kind", "subject_key") DO NOTHING;

-- Pairing requests now point at ingress rule ids.
UPDATE "channel_pairing_requests" p
SET config_id = 'ir_' || p.config_id
WHERE p.provider = 'telegram'
  AND EXISTS (
      SELECT 1 FROM "channel_configs" cc
      WHERE cc.id = p.config_id AND cc.provider = 'telegram'
  );

-- Drop legacy Slack/Telegram state after successful backfill.
DELETE FROM "channel_threads"
WHERE config_id IN (
    SELECT id FROM "channel_configs" WHERE provider IN ('slack', 'telegram')
);

-- Clear cross-provider response_config_id references that would block the delete.
UPDATE "channel_configs"
SET response_config_id = NULL
WHERE response_config_id IN (
    SELECT id FROM "channel_configs" WHERE provider IN ('slack', 'telegram')
);

DELETE FROM "channel_configs" WHERE provider IN ('slack', 'telegram');
