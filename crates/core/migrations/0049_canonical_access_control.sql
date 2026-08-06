-- Canonical access-control fields for root scoped resources.

BEGIN;

-- Reserved team for global/system rows.
INSERT INTO "teams" (
    "id", "name", "slug", "description", "is_default", "settings", "created_at", "updated_at"
)
VALUES (
    'system',
    'System',
    'system',
    'Reserved team for global system resources',
    false,
    '{}'::jsonb,
    now(),
    now()
)
ON CONFLICT ("id") DO NOTHING;

-- Ensure fallback personal team rows exist for every legacy owner reference
-- before team_id backfills run. Membership rows are added only for real users
-- below because team_members.user_id has a FK to users.
WITH owners(owner_id) AS (
    SELECT "created_by" FROM "repositories" WHERE "created_by" IS NOT NULL
    UNION
    SELECT "created_by" FROM "tasks" WHERE "created_by" IS NOT NULL
    UNION
    SELECT "created_by" FROM "task_templates" WHERE "created_by" IS NOT NULL
    UNION
    SELECT "created_by" FROM "provider_connections" WHERE "created_by" IS NOT NULL
    UNION
    SELECT "created_by" FROM "task_routes" WHERE "created_by" IS NOT NULL
    UNION
    SELECT "created_by" FROM "task_triggers" WHERE "created_by" IS NOT NULL
    UNION
    SELECT "created_by" FROM "scheduled_jobs" WHERE "created_by" IS NOT NULL
    UNION
    SELECT "scope_id" FROM "secrets" WHERE "scope_type" = 'user' AND "scope_id" IS NOT NULL
    UNION
    SELECT "user_id" FROM "provider_tokens" WHERE "user_id" IS NOT NULL
)
INSERT INTO "teams" (
    "id", "name", "slug", "description", "is_default", "settings", "created_at", "updated_at"
)
SELECT
    'personal-' || owner_id,
    owner_id || ' Personal',
    'personal-' || substr(md5(owner_id), 1, 16),
    'Personal workspace',
    false,
    '{}'::jsonb,
    now(),
    now()
FROM owners
ON CONFLICT ("id") DO NOTHING;

-- Personal teams for users that do not belong to any team.
INSERT INTO "teams" (
    "id", "name", "slug", "description", "is_default", "settings", "created_at", "updated_at"
)
SELECT
    'personal-' || u.id,
    COALESCE(NULLIF(u.login, ''), u.id) || ' Personal',
    'personal-' || substr(md5(u.id), 1, 16),
    'Personal workspace',
    false,
    '{}'::jsonb,
    now(),
    now()
FROM "users" u
WHERE NOT EXISTS (
    SELECT 1 FROM "team_members" tm WHERE tm.user_id = u.id
)
ON CONFLICT ("id") DO NOTHING;

INSERT INTO "team_members" ("team_id", "user_id", "role", "created_at")
SELECT 'personal-' || u.id, u.id, 'owner', now()
FROM "users" u
WHERE NOT EXISTS (
    SELECT 1 FROM "team_members" tm WHERE tm.user_id = u.id
)
ON CONFLICT ("team_id", "user_id") DO UPDATE SET "role" = EXCLUDED."role";

-- ---------------------------------------------------------------------------
-- Repositories: remove visibility, add owner_user_id.
-- ---------------------------------------------------------------------------

ALTER TABLE "repositories" ADD COLUMN IF NOT EXISTS "owner_user_id" text;

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'repositories' AND column_name = 'visibility'
    ) THEN
        UPDATE "repositories"
        SET "scope" = CASE
            WHEN "visibility" = 'public' THEN 'global'
            ELSE COALESCE("scope", 'user')
        END;
    END IF;
END $$;

UPDATE "repositories"
SET "scope" = COALESCE("scope", 'user');

UPDATE "repositories"
SET "team_id" = CASE
    WHEN "scope" = 'global' THEN 'system'
    WHEN "team_id" IS NOT NULL THEN "team_id"
    WHEN "created_by" IS NOT NULL THEN COALESCE(
        (SELECT tm."team_id" FROM "team_members" tm WHERE tm."user_id" = "repositories"."created_by" LIMIT 1),
        'personal-' || "created_by"
    )
    ELSE 'system'
END;

UPDATE "repositories"
SET "owner_user_id" = CASE
    WHEN "scope" = 'user' THEN COALESCE("owner_user_id", "created_by")
    ELSE NULL
END;

DROP INDEX IF EXISTS "repositories_visibility_idx";
ALTER TABLE "repositories" DROP COLUMN IF EXISTS "visibility";

-- ---------------------------------------------------------------------------
-- Tasks.
-- ---------------------------------------------------------------------------

ALTER TABLE "tasks" ADD COLUMN IF NOT EXISTS "owner_user_id" text;

UPDATE "tasks"
SET "scope" = COALESCE("scope", 'user');

UPDATE "tasks"
SET "team_id" = CASE
    WHEN "scope" = 'global' THEN 'system'
    WHEN "team_id" IS NOT NULL THEN "team_id"
    WHEN "created_by" IS NOT NULL THEN COALESCE(
        (SELECT tm."team_id" FROM "team_members" tm WHERE tm."user_id" = "tasks"."created_by" LIMIT 1),
        'personal-' || "created_by"
    )
    ELSE 'system'
END;

UPDATE "tasks"
SET "owner_user_id" = CASE
    WHEN "scope" = 'user' THEN COALESCE("owner_user_id", "created_by")
    ELSE NULL
END;

-- ---------------------------------------------------------------------------
-- Task templates.
-- ---------------------------------------------------------------------------

ALTER TABLE "task_templates" ADD COLUMN IF NOT EXISTS "owner_user_id" text;

UPDATE "task_templates"
SET "team_id" = CASE
    WHEN "scope" = 'global' THEN 'system'
    WHEN "team_id" IS NOT NULL THEN "team_id"
    WHEN "created_by" IS NOT NULL THEN COALESCE(
        (SELECT tm."team_id" FROM "team_members" tm WHERE tm."user_id" = "task_templates"."created_by" LIMIT 1),
        'personal-' || "created_by"
    )
    ELSE 'system'
END;

UPDATE "task_templates"
SET "owner_user_id" = CASE
    WHEN "scope" = 'user' THEN COALESCE("owner_user_id", "created_by")
    ELSE NULL
END;

-- ---------------------------------------------------------------------------
-- Provider connections, routes, triggers, and scheduled jobs.
-- ---------------------------------------------------------------------------

ALTER TABLE "provider_connections" ADD COLUMN IF NOT EXISTS "owner_user_id" text;
ALTER TABLE "task_routes" ADD COLUMN IF NOT EXISTS "owner_user_id" text;
ALTER TABLE "task_triggers" ADD COLUMN IF NOT EXISTS "owner_user_id" text;
ALTER TABLE "scheduled_jobs" ADD COLUMN IF NOT EXISTS "owner_user_id" text;

UPDATE "provider_connections"
SET "team_id" = CASE
    WHEN "scope" = 'global' THEN 'system'
    WHEN "team_id" IS NOT NULL THEN "team_id"
    ELSE COALESCE(
        (SELECT tm."team_id" FROM "team_members" tm WHERE tm."user_id" = "provider_connections"."created_by" LIMIT 1),
        'personal-' || "created_by"
    )
END,
"owner_user_id" = CASE
    WHEN "scope" = 'user' THEN COALESCE("owner_user_id", "created_by")
    ELSE NULL
END;

UPDATE "task_routes"
SET "team_id" = CASE
    WHEN "scope" = 'global' THEN 'system'
    WHEN "team_id" IS NOT NULL THEN "team_id"
    ELSE COALESCE(
        (SELECT tm."team_id" FROM "team_members" tm WHERE tm."user_id" = "task_routes"."created_by" LIMIT 1),
        'personal-' || "created_by"
    )
END,
"owner_user_id" = CASE
    WHEN "scope" = 'user' THEN COALESCE("owner_user_id", "created_by")
    ELSE NULL
END;

UPDATE "task_triggers"
SET "team_id" = CASE
    WHEN "scope" = 'global' THEN 'system'
    WHEN "team_id" IS NOT NULL THEN "team_id"
    ELSE COALESCE(
        (SELECT tm."team_id" FROM "team_members" tm WHERE tm."user_id" = "task_triggers"."created_by" LIMIT 1),
        'personal-' || "created_by"
    )
END,
"owner_user_id" = CASE
    WHEN "scope" = 'user' THEN COALESCE("owner_user_id", "created_by")
    ELSE NULL
END;

UPDATE "scheduled_jobs"
SET "team_id" = CASE
    WHEN "scope" = 'global' THEN 'system'
    WHEN "team_id" IS NOT NULL THEN "team_id"
    ELSE COALESCE(
        (SELECT tm."team_id" FROM "team_members" tm WHERE tm."user_id" = "scheduled_jobs"."created_by" LIMIT 1),
        'personal-' || "created_by"
    )
END,
"owner_user_id" = CASE
    WHEN "scope" = 'user' THEN COALESCE("owner_user_id", "created_by")
    ELSE NULL
END;

-- ---------------------------------------------------------------------------
-- Secrets: add canonical columns, keep scope_type/scope_id for compatibility.
-- ---------------------------------------------------------------------------

ALTER TABLE "secrets" ADD COLUMN IF NOT EXISTS "scope" text;
ALTER TABLE "secrets" ADD COLUMN IF NOT EXISTS "team_id" text;
ALTER TABLE "secrets" ADD COLUMN IF NOT EXISTS "owner_user_id" text;

UPDATE "secrets"
SET "scope" = COALESCE("scope", "scope_type"),
    "team_id" = CASE
        WHEN COALESCE("scope", "scope_type") = 'global' THEN 'system'
        WHEN COALESCE("scope", "scope_type") = 'team' THEN "scope_id"
        WHEN COALESCE("scope", "scope_type") = 'user' THEN COALESCE(
            (SELECT tm."team_id" FROM "team_members" tm WHERE tm."user_id" = "secrets"."scope_id" LIMIT 1),
            'personal-' || "scope_id"
        )
        ELSE 'system'
    END,
    "owner_user_id" = CASE
        WHEN COALESCE("scope", "scope_type") = 'user' THEN "scope_id"
        ELSE NULL
    END;

-- ---------------------------------------------------------------------------
-- Provider tokens.
-- ---------------------------------------------------------------------------

DO $$
BEGIN
    IF EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'provider_tokens' AND column_name = 'scope'
    ) AND NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'provider_tokens' AND column_name = 'oauth_scope'
    ) THEN
        ALTER TABLE "provider_tokens" RENAME COLUMN "scope" TO "oauth_scope";
    END IF;
END $$;

ALTER TABLE "provider_tokens" ADD COLUMN IF NOT EXISTS "scope" text;
ALTER TABLE "provider_tokens" ADD COLUMN IF NOT EXISTS "owner_user_id" text;

UPDATE "provider_tokens"
SET "scope" = CASE
        WHEN "team_id" IS NOT NULL AND "user_id" IS NULL THEN 'team'
        WHEN "user_id" IS NOT NULL THEN 'user'
        ELSE COALESCE("scope", 'global')
    END;

UPDATE "provider_tokens"
SET "team_id" = CASE
        WHEN "scope" = 'global' THEN 'system'
        WHEN "team_id" IS NOT NULL THEN "team_id"
        WHEN "user_id" IS NOT NULL THEN COALESCE(
            (SELECT tm."team_id" FROM "team_members" tm WHERE tm."user_id" = "provider_tokens"."user_id" LIMIT 1),
            'personal-' || "user_id"
        )
        ELSE 'system'
    END,
    "owner_user_id" = CASE
        WHEN "scope" = 'user' THEN COALESCE("owner_user_id", "user_id")
        ELSE NULL
    END;

-- ---------------------------------------------------------------------------
-- Commands, plugin repositories, and file metadata.
-- ---------------------------------------------------------------------------

ALTER TABLE "commands" ADD COLUMN IF NOT EXISTS "scope" text NOT NULL DEFAULT 'global';
ALTER TABLE "commands" ADD COLUMN IF NOT EXISTS "team_id" text NOT NULL DEFAULT 'system';
ALTER TABLE "commands" ADD COLUMN IF NOT EXISTS "owner_user_id" text;
ALTER TABLE "commands" ADD COLUMN IF NOT EXISTS "created_by" text DEFAULT 'system';

ALTER TABLE "plugin_repos" ADD COLUMN IF NOT EXISTS "scope" text NOT NULL DEFAULT 'global';
ALTER TABLE "plugin_repos" ADD COLUMN IF NOT EXISTS "team_id" text NOT NULL DEFAULT 'system';
ALTER TABLE "plugin_repos" ADD COLUMN IF NOT EXISTS "owner_user_id" text;
ALTER TABLE "plugin_repos" ADD COLUMN IF NOT EXISTS "created_by" text DEFAULT 'system';

ALTER TABLE "file_metadata" ADD COLUMN IF NOT EXISTS "scope" text NOT NULL DEFAULT 'global';
ALTER TABLE "file_metadata" ADD COLUMN IF NOT EXISTS "team_id" text NOT NULL DEFAULT 'system';
ALTER TABLE "file_metadata" ADD COLUMN IF NOT EXISTS "owner_user_id" text;
ALTER TABLE "file_metadata" ADD COLUMN IF NOT EXISTS "created_by" text DEFAULT 'system';

-- ---------------------------------------------------------------------------
-- Constraints and indexes. Add constraints idempotently with DO blocks.
-- ---------------------------------------------------------------------------

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'repositories_scope_chk') THEN
        ALTER TABLE "repositories" ADD CONSTRAINT "repositories_scope_chk"
            CHECK ("scope" IN ('global', 'team', 'user'));
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'repositories_owner_scope_chk') THEN
        ALTER TABLE "repositories" ADD CONSTRAINT "repositories_owner_scope_chk"
            CHECK (
                ("scope" = 'user' AND "owner_user_id" IS NOT NULL)
                OR ("scope" IN ('global', 'team') AND "owner_user_id" IS NULL)
            );
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'tasks_owner_scope_chk') THEN
        ALTER TABLE "tasks" ADD CONSTRAINT "tasks_owner_scope_chk"
            CHECK (
                ("scope" = 'user' AND "owner_user_id" IS NOT NULL)
                OR ("scope" IN ('global', 'team') AND "owner_user_id" IS NULL)
            );
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'file_metadata_owner_scope_chk') THEN
        ALTER TABLE "file_metadata" ADD CONSTRAINT "file_metadata_owner_scope_chk"
            CHECK (
                ("scope" = 'user' AND "owner_user_id" IS NOT NULL)
                OR ("scope" IN ('global', 'team') AND "owner_user_id" IS NULL)
            );
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'provider_connections_owner_scope_chk') THEN
        ALTER TABLE "provider_connections" ADD CONSTRAINT "provider_connections_owner_scope_chk"
            CHECK (
                ("scope" = 'user' AND "owner_user_id" IS NOT NULL)
                OR ("scope" IN ('global', 'team') AND "owner_user_id" IS NULL)
            );
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'task_routes_owner_scope_chk') THEN
        ALTER TABLE "task_routes" ADD CONSTRAINT "task_routes_owner_scope_chk"
            CHECK (
                ("scope" = 'user' AND "owner_user_id" IS NOT NULL)
                OR ("scope" IN ('global', 'team') AND "owner_user_id" IS NULL)
            );
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'task_triggers_owner_scope_chk') THEN
        ALTER TABLE "task_triggers" ADD CONSTRAINT "task_triggers_owner_scope_chk"
            CHECK (
                ("scope" = 'user' AND "owner_user_id" IS NOT NULL)
                OR ("scope" IN ('global', 'team') AND "owner_user_id" IS NULL)
            );
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'scheduled_jobs_owner_scope_chk') THEN
        ALTER TABLE "scheduled_jobs" ADD CONSTRAINT "scheduled_jobs_owner_scope_chk"
            CHECK (
                ("scope" = 'user' AND "owner_user_id" IS NOT NULL)
                OR ("scope" IN ('global', 'team') AND "owner_user_id" IS NULL)
            );
    END IF;
END $$;

ALTER TABLE "repositories" ALTER COLUMN "team_id" SET NOT NULL;
ALTER TABLE "tasks" ALTER COLUMN "team_id" SET NOT NULL;
ALTER TABLE "task_templates" ALTER COLUMN "team_id" SET NOT NULL;
ALTER TABLE "provider_connections" ALTER COLUMN "team_id" SET NOT NULL;
ALTER TABLE "task_routes" ALTER COLUMN "team_id" SET NOT NULL;
ALTER TABLE "task_triggers" ALTER COLUMN "team_id" SET NOT NULL;
ALTER TABLE "scheduled_jobs" ALTER COLUMN "team_id" SET NOT NULL;
ALTER TABLE "secrets" ALTER COLUMN "scope" SET NOT NULL;
ALTER TABLE "secrets" ALTER COLUMN "team_id" SET NOT NULL;
ALTER TABLE "provider_tokens" ALTER COLUMN "scope" SET NOT NULL;
ALTER TABLE "provider_tokens" ALTER COLUMN "team_id" SET NOT NULL;

CREATE INDEX IF NOT EXISTS "repositories_scope_team_owner_idx"
    ON "repositories" ("scope", "team_id", "owner_user_id");
CREATE INDEX IF NOT EXISTS "tasks_scope_team_owner_idx"
    ON "tasks" ("scope", "team_id", "owner_user_id");
CREATE INDEX IF NOT EXISTS "file_metadata_scope_team_owner_idx"
    ON "file_metadata" ("scope", "team_id", "owner_user_id");
CREATE INDEX IF NOT EXISTS "provider_connections_scope_team_owner_idx"
    ON "provider_connections" ("scope", "team_id", "owner_user_id");
CREATE INDEX IF NOT EXISTS "task_routes_scope_team_owner_idx"
    ON "task_routes" ("scope", "team_id", "owner_user_id");
CREATE INDEX IF NOT EXISTS "task_triggers_scope_team_owner_idx"
    ON "task_triggers" ("scope", "team_id", "owner_user_id");
CREATE INDEX IF NOT EXISTS "scheduled_jobs_scope_team_owner_idx"
    ON "scheduled_jobs" ("scope", "team_id", "owner_user_id");
CREATE INDEX IF NOT EXISTS "secrets_canonical_scope_idx"
    ON "secrets" ("scope", "team_id", "owner_user_id");

COMMIT;
