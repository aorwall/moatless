-- GitHub App registrations as durable, administered state.
--
-- Before this, a bot App lived only in the process environment
-- (`GITHUB_APPS_JSON`) and could only change with a rollout. The App used for
-- signing in is deliberately NOT stored here: it stays deployment-configured,
-- so that nothing done through the UI can make the UI unreachable.
--
-- The signing key is encrypted with the same envelope cipher the `secrets`
-- table uses. It is stored on the record rather than as a `secrets` row so it
-- cannot be renamed, orphaned or deleted independently of the App that needs
-- it. `id` is the additional authenticated data for the value ciphertext,
-- exactly as `secrets.id` is for a secret.

CREATE TABLE IF NOT EXISTS "github_apps" (
    "id" text PRIMARY KEY NOT NULL,
    -- Short stable name the App is referenced by: bot users store it, and
    -- Adapter connections name it in their transport config. Case-sensitive,
    -- because existing bot users already reference keys verbatim.
    "app_key" text NOT NULL,
    -- GitHub's own numeric id for the App.
    "app_id" bigint NOT NULL,
    -- Resolved from GitHub when the App was proved, and refreshed on rotation.
    "name" text NOT NULL,
    "slug" text NOT NULL,
    "bot_login" text NOT NULL,
    -- Envelope-encrypted signing key. Never returned by any read path.
    "private_key_ciphertext" bytea NOT NULL,
    "private_key_nonce" bytea NOT NULL,
    "private_key_data_key" bytea NOT NULL,
    "kms_key_id" text NOT NULL,
    "encryption_version" integer NOT NULL DEFAULT 1,
    -- Non-reversing fingerprint, so two keys can be told apart in a UI that
    -- must never show either.
    "key_fingerprint" text NOT NULL,
    "default_installation_id" bigint,
    "created_by" text NOT NULL,
    "updated_by" text NOT NULL,
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "updated_at" timestamptz NOT NULL DEFAULT now(),
    -- Bumped on every write. Backends hold obtained GitHub tokens against a
    -- revision, so editing one App discards only that App's tokens.
    "revision" integer NOT NULL DEFAULT 1,
    CONSTRAINT "github_apps_key_format_chk"
        CHECK ("app_key" ~ '^[A-Za-z0-9][A-Za-z0-9_-]*$'),
    CONSTRAINT "github_apps_app_id_chk" CHECK ("app_id" > 0),
    CONSTRAINT "github_apps_revision_chk" CHECK ("revision" > 0)
);

CREATE UNIQUE INDEX IF NOT EXISTS "github_apps_key_uq"
    ON "github_apps" ("app_key");

-- Who changed which App, and when. The `secrets` table gets this for free from
-- `secret_audit_events`; a key stored on its own record does not.
CREATE TABLE IF NOT EXISTS "github_app_audit_events" (
    "id" text PRIMARY KEY NOT NULL,
    "github_app_id" text,
    "app_key" text NOT NULL,
    "actor_id" text NOT NULL,
    "action" text NOT NULL,
    "result" text NOT NULL,
    "detail" text,
    "created_at" timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS "github_app_audit_events_app_idx"
    ON "github_app_audit_events" ("github_app_id", "created_at");

CREATE INDEX IF NOT EXISTS "github_app_audit_events_actor_idx"
    ON "github_app_audit_events" ("actor_id", "created_at");

-- Marks the one-time carry-over of `GITHUB_APPS_JSON` into `github_apps`.
--
-- Whether the carry-over has run is recorded here in its own right, never
-- inferred from which Apps currently exist: inferring it would resurrect a
-- carried-over App that an admin has since deleted, at the next restart.
-- Claiming the row with `ON CONFLICT DO NOTHING` is also what makes several
-- backends starting together produce one outcome.
CREATE TABLE IF NOT EXISTS "github_app_migrations" (
    "name" text PRIMARY KEY NOT NULL,
    "completed_at" timestamptz NOT NULL DEFAULT now(),
    "detail" text
);
