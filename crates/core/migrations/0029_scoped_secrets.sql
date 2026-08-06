-- Scoped, ciphertext-only application secrets.

CREATE TABLE IF NOT EXISTS "secrets" (
    "id" text PRIMARY KEY NOT NULL,
    "scope_type" text NOT NULL,
    "scope_id" text,
    "key" text NOT NULL,
    "kind" text NOT NULL,
    "provider" text,
    "target" text NOT NULL,
    "value_ciphertext" bytea,
    "encryption_nonce" bytea,
    "encrypted_data_key" bytea,
    "external_ref" text,
    "kms_key_id" text,
    "encryption_version" integer NOT NULL DEFAULT 1,
    "created_by" text NOT NULL,
    "updated_by" text NOT NULL,
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "updated_at" timestamptz NOT NULL DEFAULT now(),
    "last_used_at" timestamptz,
    "version" integer NOT NULL DEFAULT 1,
    CONSTRAINT "secrets_scope_type_chk"
        CHECK ("scope_type" IN ('global', 'team', 'user')),
    CONSTRAINT "secrets_scope_id_chk"
        CHECK (
            ("scope_type" = 'global' AND "scope_id" IS NULL)
            OR ("scope_type" IN ('team', 'user') AND "scope_id" IS NOT NULL)
        ),
    CONSTRAINT "secrets_kind_chk"
        CHECK ("kind" IN ('env', 'provider_token', 'runtime_file', 'deployment')),
    CONSTRAINT "secrets_target_chk"
        CHECK (
            "target" IN ('agent', 'all_servers', 'all', 'backend')
            OR "target" LIKE 'server:%'
        ),
    CONSTRAINT "secrets_key_format_chk"
        CHECK ("key" ~ '^[A-Z_][A-Z0-9_]*$'),
    CONSTRAINT "secrets_source_chk"
        CHECK (
            ("value_ciphertext" IS NOT NULL AND "external_ref" IS NULL)
            OR ("value_ciphertext" IS NULL AND "external_ref" IS NOT NULL)
        ),
    CONSTRAINT "secrets_encryption_metadata_chk"
        CHECK (
            "value_ciphertext" IS NULL
            OR (
                "encryption_nonce" IS NOT NULL
                AND "encrypted_data_key" IS NOT NULL
                AND "kms_key_id" IS NOT NULL
                AND "encryption_version" > 0
            )
        ),
    CONSTRAINT "secrets_version_chk" CHECK ("version" > 0)
);

CREATE UNIQUE INDEX IF NOT EXISTS "secrets_scope_key_target_uq"
    ON "secrets" ("scope_type", COALESCE("scope_id", ''), "key", "target");

CREATE INDEX IF NOT EXISTS "secrets_scope_idx"
    ON "secrets" ("scope_type", "scope_id");

CREATE TABLE IF NOT EXISTS "secret_audit_events" (
    "id" text PRIMARY KEY NOT NULL,
    "secret_id" text,
    "actor_id" text NOT NULL,
    "action" text NOT NULL,
    "scope_type" text NOT NULL,
    "scope_id" text,
    "key" text NOT NULL,
    "target" text NOT NULL,
    "result" text NOT NULL,
    "created_at" timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS "secret_audit_events_secret_idx"
    ON "secret_audit_events" ("secret_id", "created_at");

CREATE INDEX IF NOT EXISTS "secret_audit_events_actor_idx"
    ON "secret_audit_events" ("actor_id", "created_at");
