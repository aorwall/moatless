-- =========================================================================
-- Workspace concept — Phase A (additive + backfill, no behavior change)
-- =========================================================================
-- Introduces a first-class Workspace entity that owns the run-configuration
-- and holds 0..n repository references. A Repository stays a reusable git
-- pointer. Tasks gain a nullable `workspace_id`.
--
-- Backfill creates a 1:1 "shadow" workspace for every existing repository
-- (copying its run-config columns) and points every existing task at its
-- repository's shadow workspace. This is mechanical and reversible: a repo and
-- its shadow workspace stay 1:1 until someone edits them apart.
--
-- Phase A only: provisioning still reads the *repository* row for config
-- (byte-for-byte identical single-repo path). It resolves *which* repository
-- via the workspace when `tasks.workspace_id` is set, else the legacy
-- `tasks.repository_id`. The copied run-config columns below are populated for
-- the forthcoming phases that move config-reading onto the Workspace.

-- -------------------------------------------------------------------------
-- workspaces — the run configuration + composition
-- -------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS "workspaces" (
    "id" text PRIMARY KEY NOT NULL,
    "name" text NOT NULL,
    "description" text,
    -- Derived/validated label from the WorkspaceRepo count: blank | single | multi.
    "kind" text NOT NULL DEFAULT 'single',
    -- Run configuration (mirrors the repositories run-config columns).
    "docker_image" text,
    "docker_access" boolean,
    "sandbox_service_account" text,
    "system_prompt" text,
    "setup_commands" jsonb,
    "env_vars" jsonb,
    "servers" jsonb DEFAULT '[]'::jsonb,
    "plugins" jsonb,
    "resources" jsonb,
    -- Access control (mirrors repositories: scope / team_id / owner_user_id).
    "scope" text NOT NULL DEFAULT 'user',
    "team_id" text,
    "owner_user_id" text,
    "created_by" text,
    "deleted" boolean,
    "created_at" timestamptz DEFAULT now(),
    "updated_at" timestamptz DEFAULT now()
);

CREATE INDEX IF NOT EXISTS "workspaces_created_by_idx" ON "workspaces" USING btree ("created_by");
CREATE INDEX IF NOT EXISTS "workspaces_scope_idx" ON "workspaces" USING btree ("scope");

-- -------------------------------------------------------------------------
-- workspace_repos — a repository's placement inside a workspace (0..n)
-- -------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS "workspace_repos" (
    "id" text PRIMARY KEY NOT NULL,
    "workspace_id" text NOT NULL REFERENCES "workspaces"("id") ON DELETE CASCADE,
    "repository_id" text NOT NULL,
    -- Override repo.default_branch for this placement (NULL = use repo default).
    "branch" text,
    -- Subdir under the workspace dir (NULL = default to the repo name at runtime).
    "mount_name" text,
    -- The "main" repo for diff/PR/AGENTS.md. Exactly one per workspace when n>=1.
    "is_primary" boolean NOT NULL DEFAULT false,
    -- Ordering within the workspace.
    "position" integer NOT NULL DEFAULT 0,
    "created_at" timestamptz DEFAULT now()
);

CREATE INDEX IF NOT EXISTS "workspace_repos_workspace_idx" ON "workspace_repos" USING btree ("workspace_id");
CREATE INDEX IF NOT EXISTS "workspace_repos_repository_idx" ON "workspace_repos" USING btree ("repository_id");
-- Per-workspace mount-name uniqueness (only enforced when a mount name is set).
CREATE UNIQUE INDEX IF NOT EXISTS "workspace_repos_mount_unique"
    ON "workspace_repos" ("workspace_id", "mount_name") WHERE "mount_name" IS NOT NULL;
-- At most one primary repo per workspace.
CREATE UNIQUE INDEX IF NOT EXISTS "workspace_repos_one_primary"
    ON "workspace_repos" ("workspace_id") WHERE "is_primary";

-- -------------------------------------------------------------------------
-- tasks.workspace_id — nullable during the additive transition
-- -------------------------------------------------------------------------
ALTER TABLE "tasks" ADD COLUMN IF NOT EXISTS "workspace_id" text;
CREATE INDEX IF NOT EXISTS "tasks_workspace_idx" ON "tasks" USING btree ("workspace_id");

-- -------------------------------------------------------------------------
-- Backfill: one shadow workspace per repository (id = 'ws_' || repository id)
-- -------------------------------------------------------------------------
INSERT INTO "workspaces" (
    "id", "name", "description", "kind",
    "docker_image", "docker_access", "sandbox_service_account",
    "system_prompt", "setup_commands", "env_vars", "servers", "plugins", "resources",
    "scope", "team_id", "owner_user_id", "created_by", "deleted", "created_at", "updated_at"
)
SELECT
    'ws_' || r."id", r."name", r."description", 'single',
    r."docker_image", r."docker_access", r."sandbox_service_account",
    r."system_prompt", r."setup_commands", r."env_vars",
    COALESCE(r."servers", '[]'::jsonb), r."plugins", r."resources",
    COALESCE(r."scope", 'user'), r."team_id", r."owner_user_id", r."created_by", r."deleted",
    COALESCE(r."created_at", now()), COALESCE(r."updated_at", now())
FROM "repositories" r
ON CONFLICT ("id") DO NOTHING;

-- Backfill: one primary workspace_repo per shadow workspace.
INSERT INTO "workspace_repos" (
    "id", "workspace_id", "repository_id", "branch", "mount_name", "is_primary", "position"
)
SELECT
    'wsr_' || r."id", 'ws_' || r."id", r."id", NULL, NULL, true, 0
FROM "repositories" r
ON CONFLICT ("id") DO NOTHING;

-- Backfill: point every task at its repository's shadow workspace.
UPDATE "tasks" t
SET "workspace_id" = 'ws_' || t."repository_id"
WHERE t."workspace_id" IS NULL
  AND EXISTS (SELECT 1 FROM "workspaces" w WHERE w."id" = 'ws_' || t."repository_id");
