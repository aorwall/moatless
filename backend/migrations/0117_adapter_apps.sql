-- Generic Adapter app/API configuration.
--
-- Adapter apps are deployment-wide credentials and public settings for an
-- Adapter integration, such as GitHub bot Apps, Slack bot credentials, Linear
-- OAuth/webhook settings, and the single Telegram bot. Authentication provider
-- configuration stays deployment-controlled.

CREATE TABLE IF NOT EXISTS "adapter_apps" (
    "id" text PRIMARY KEY NOT NULL,
    "adapter_kind" text NOT NULL,
    "app_key" text NOT NULL,
    "display_name" text NOT NULL,
    "enabled" boolean NOT NULL DEFAULT true,
    "config_json" jsonb NOT NULL DEFAULT '{}'::jsonb,
    "secret_fingerprints" jsonb NOT NULL DEFAULT '{}'::jsonb,
    "created_by" text NOT NULL,
    "updated_by" text NOT NULL,
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "updated_at" timestamptz NOT NULL DEFAULT now(),
    "revision" integer NOT NULL DEFAULT 1,
    CONSTRAINT "adapter_apps_kind_format_chk"
        CHECK ("adapter_kind" ~ '^[a-z][a-z0-9_]*$'),
    CONSTRAINT "adapter_apps_key_format_chk"
        CHECK ("app_key" ~ '^[A-Za-z0-9][A-Za-z0-9_-]*$'),
    CONSTRAINT "adapter_apps_revision_chk" CHECK ("revision" > 0)
);

CREATE UNIQUE INDEX IF NOT EXISTS "adapter_apps_kind_key_uq"
    ON "adapter_apps" ("adapter_kind", "app_key");

CREATE INDEX IF NOT EXISTS "adapter_apps_kind_idx"
    ON "adapter_apps" ("adapter_kind", "enabled", "app_key");

CREATE TABLE IF NOT EXISTS "adapter_app_secrets" (
    "adapter_app_id" text NOT NULL REFERENCES "adapter_apps" ("id") ON DELETE CASCADE,
    "name" text NOT NULL,
    "value_ciphertext" bytea NOT NULL,
    "value_nonce" bytea NOT NULL,
    "encrypted_data_key" bytea NOT NULL,
    "kms_key_id" text NOT NULL,
    "encryption_version" integer NOT NULL DEFAULT 1,
    "updated_at" timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY ("adapter_app_id", "name"),
    CONSTRAINT "adapter_app_secrets_name_format_chk"
        CHECK ("name" ~ '^[a-z][a-z0-9_]*$')
);

CREATE TABLE IF NOT EXISTS "adapter_app_audit_events" (
    "id" text PRIMARY KEY NOT NULL,
    "adapter_app_id" text,
    "adapter_kind" text NOT NULL,
    "app_key" text NOT NULL,
    "actor_id" text NOT NULL,
    "action" text NOT NULL,
    "result" text NOT NULL,
    "detail" text,
    "created_at" timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS "adapter_app_audit_events_app_idx"
    ON "adapter_app_audit_events" ("adapter_kind", "app_key", "created_at");

CREATE INDEX IF NOT EXISTS "adapter_app_audit_events_actor_idx"
    ON "adapter_app_audit_events" ("actor_id", "created_at");

-- Generic one-time migration markers. Completion is recorded explicitly so a
-- deleted carried-over app is not recreated by a later restart.
CREATE TABLE IF NOT EXISTS "adapter_app_migrations" (
    "name" text PRIMARY KEY NOT NULL,
    "completed_at" timestamptz NOT NULL DEFAULT now(),
    "detail" text
);

-- Move existing admin-registered GitHub bot Apps into the generic table while
-- keeping github_apps as rollback state for this release. The encrypted key
-- remains decryptable because the generic row keeps the same id, and the id is
-- the additional authenticated data for the ciphertext.
INSERT INTO "adapter_apps" (
    "id",
    "adapter_kind",
    "app_key",
    "display_name",
    "enabled",
    "config_json",
    "secret_fingerprints",
    "created_by",
    "updated_by",
    "created_at",
    "updated_at",
    "revision"
)
SELECT
    "id",
    'github',
    "app_key",
    "name",
    true,
    jsonb_strip_nulls(jsonb_build_object(
        'app_id', "app_id",
        'name', "name",
        'slug', "slug",
        'bot_login', "bot_login",
        'default_installation_id', "default_installation_id"
    )),
    jsonb_build_object('private_key', "key_fingerprint"),
    "created_by",
    "updated_by",
    "created_at",
    "updated_at",
    "revision"
FROM "github_apps"
ON CONFLICT ("adapter_kind", "app_key") DO NOTHING;

INSERT INTO "adapter_app_secrets" (
    "adapter_app_id",
    "name",
    "value_ciphertext",
    "value_nonce",
    "encrypted_data_key",
    "kms_key_id",
    "encryption_version",
    "updated_at"
)
SELECT
    "id",
    'private_key',
    "private_key_ciphertext",
    "private_key_nonce",
    "private_key_data_key",
    "kms_key_id",
    "encryption_version",
    "updated_at"
FROM "github_apps"
ON CONFLICT ("adapter_app_id", "name") DO NOTHING;

INSERT INTO "adapter_app_migrations" ("name", "detail")
SELECT 'github_apps_table_import_v1', imported.count::text || ' GitHub App row(s) imported'
FROM (SELECT count(*) FROM "github_apps") imported
WHERE imported.count > 0
ON CONFLICT ("name") DO NOTHING;
