-- The identity a Workspace's Tasks act as on a git host.
--
-- A Workspace names the account it runs as, so it outranks whoever started the
-- Task. That is the precedence `workspace_credentials` already has for GitHub;
-- this column moves the subject from a stored secret to a person, so the
-- credential is still derived from an identity rather than pinned beside one.
--
-- Nullable: a Workspace that names nobody leaves every Task acting as its own
-- owner, which is the ordinary case.
--
-- `text` holding a `users.id` uuid and no foreign key — the shape
-- `loop_configs.run_as_user_id` and this table's own `owner_user_id` already
-- have. A named user that no longer exists resolves to no credential, so the
-- git-host call refuses by name instead of acting as somebody else.

ALTER TABLE "workspaces"
    ADD COLUMN IF NOT EXISTS "run_as_user_id" text;

CREATE INDEX IF NOT EXISTS "workspaces_run_as_user_id_idx"
    ON "workspaces" ("run_as_user_id")
    WHERE "run_as_user_id" IS NOT NULL;
