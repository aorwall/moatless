-- Access Control: Global / Team / User Scope System
-- New tables: teams, team_members
-- Alter existing: provider_tokens, repositories, tasks, channel_configs, sessions, api_keys

-- =============================================
-- New tables
-- =============================================

CREATE TABLE IF NOT EXISTS "teams" (
    "id" text PRIMARY KEY NOT NULL,
    "name" text NOT NULL,
    "slug" text NOT NULL UNIQUE,
    "description" text,
    "external_id" text,
    "external_provider" text,
    "is_default" boolean NOT NULL DEFAULT false,
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "updated_at" timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS "team_members" (
    "team_id" text NOT NULL REFERENCES "teams"("id") ON DELETE CASCADE,
    "user_id" text NOT NULL REFERENCES "users"("id") ON DELETE CASCADE,
    "role" text NOT NULL DEFAULT 'member',
    "created_at" timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY ("team_id", "user_id")
);
CREATE INDEX IF NOT EXISTS "team_members_user_idx" ON "team_members" ("user_id");

-- Partial unique index for external_id + external_provider (used by upsert_by_external)
CREATE UNIQUE INDEX IF NOT EXISTS "teams_external_uq"
    ON "teams" ("external_id", "external_provider")
    WHERE "external_id" IS NOT NULL AND "external_provider" IS NOT NULL;

-- =============================================
-- Alter provider_tokens: add team_id
-- =============================================

ALTER TABLE "provider_tokens" ADD COLUMN IF NOT EXISTS "team_id" text REFERENCES "teams"("id");

-- Make user_id nullable so team-level tokens (team_id set, user_id NULL) can be stored
ALTER TABLE "provider_tokens" ALTER COLUMN "user_id" DROP NOT NULL;

-- Drop old unique constraint and add new one that includes team_id
-- Note: constraint name may vary; use DO block for safety
DO $$
BEGIN
    -- Try dropping common constraint names
    BEGIN
        ALTER TABLE "provider_tokens" DROP CONSTRAINT IF EXISTS "provider_tokens_user_provider_uq";
    EXCEPTION WHEN undefined_object THEN NULL;
    END;
    BEGIN
        ALTER TABLE "provider_tokens" DROP CONSTRAINT IF EXISTS "provider_tokens_user_id_provider_key";
    EXCEPTION WHEN undefined_object THEN NULL;
    END;
END $$;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint WHERE conname = 'provider_tokens_scope_uq'
    ) THEN
        ALTER TABLE "provider_tokens" ADD CONSTRAINT "provider_tokens_scope_uq"
            UNIQUE NULLS NOT DISTINCT ("user_id", "provider", "team_id");
    END IF;
END $$;

-- =============================================
-- Alter repositories: add scope + team_id
-- =============================================

ALTER TABLE "repositories" ADD COLUMN IF NOT EXISTS "scope" text NOT NULL DEFAULT 'user';
ALTER TABLE "repositories" ADD COLUMN IF NOT EXISTS "team_id" text REFERENCES "teams"("id");

UPDATE "repositories" SET scope = 'global' WHERE visibility = 'public';
UPDATE "repositories" SET scope = 'user' WHERE visibility = 'private' OR visibility IS NULL;

-- =============================================
-- Alter tasks: add scope + team_id
-- =============================================

ALTER TABLE "tasks" ADD COLUMN IF NOT EXISTS "scope" text NOT NULL DEFAULT 'user';
ALTER TABLE "tasks" ADD COLUMN IF NOT EXISTS "team_id" text;

UPDATE "tasks" SET scope = 'global' WHERE visibility = 'public';
UPDATE "tasks" SET scope = 'user' WHERE visibility = 'private' OR visibility IS NULL;

-- =============================================
-- Alter channel_configs: add scope + team_id + owner_id
-- =============================================

ALTER TABLE "channel_configs" ADD COLUMN IF NOT EXISTS "scope" text NOT NULL DEFAULT 'global';
ALTER TABLE "channel_configs" ADD COLUMN IF NOT EXISTS "team_id" text REFERENCES "teams"("id");
ALTER TABLE "channel_configs" ADD COLUMN IF NOT EXISTS "owner_id" text;

-- =============================================
-- Alter sessions / api_keys: add team_ids JSON
-- =============================================

ALTER TABLE "sessions" ADD COLUMN IF NOT EXISTS "team_ids" text;
ALTER TABLE "api_keys" ADD COLUMN IF NOT EXISTS "team_ids" text;
