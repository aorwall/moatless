-- Forgejo OAuth2 applications, one per Forgejo instance.
--
-- Forgejo is self-hostable, so "Forgejo" names no single host and no single
-- OAuth application. A repository's git URL names the host, and that host
-- selects the row here that its connect flow runs against. A deployment with
-- no row for a host still resolves personal access tokens for it: this table
-- gates OAuth only.
--
-- The client secret is encrypted with the same envelope cipher the `secrets`
-- table uses, stored on the record rather than as a `secrets` row so it cannot
-- be renamed, orphaned or deleted independently of the instance that needs it.
-- `id` is the additional authenticated data, exactly as `secrets.id` is for a
-- secret and `github_apps.id` is for a signing key.

CREATE TABLE IF NOT EXISTS "forgejo_instances" (
    "id" text PRIMARY KEY NOT NULL,
    -- Hostname only, lowercased, no scheme and no path: it is compared against
    -- the host parsed out of a repository's remote URL.
    "host" text NOT NULL,
    -- The OAuth application's public identifier, shown in admin screens.
    "client_id" text NOT NULL,
    -- Envelope-encrypted client secret. Never returned by any read path.
    "client_secret_ciphertext" bytea NOT NULL,
    "client_secret_nonce" bytea NOT NULL,
    "client_secret_data_key" bytea NOT NULL,
    "kms_key_id" text NOT NULL,
    "encryption_version" integer NOT NULL DEFAULT 1,
    "created_by" text NOT NULL,
    "updated_by" text NOT NULL,
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "updated_at" timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT "forgejo_instances_host_format_chk"
        CHECK ("host" ~ '^[a-z0-9][a-z0-9.-]*(:[0-9]+)?$')
);

CREATE UNIQUE INDEX IF NOT EXISTS "forgejo_instances_host_uq"
    ON "forgejo_instances" ("host");
