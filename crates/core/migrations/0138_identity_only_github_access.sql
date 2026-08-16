-- =========================================================================
-- Every GitHub credential belongs to a named identity
-- =========================================================================
-- A GitHub token is now resolved from the person who asked, the Bot user a
-- Loop runs as, or the identity a Workspace runs as. Nothing derives it from
-- an org-wide row, from a value pinned to a Workspace, or from a repository
-- row. This drops the three stores that held it otherwise.
--
-- Not reversible. Deploy the code that stops reading these first: an older
-- binary against this schema fails on the repositories columns.

-- 1) The org-wide GitHub credential. `find_github_row` and the presence query
--    both address a user id, so this row is unreachable.
DELETE FROM "provider_tokens" WHERE "provider" = 'github' AND "scope" = 'global';

-- 2) `scope` carries two values. 'team' left access control in 0097, whose
--    provider_tokens_owner_scope_chk has refused it since; state the legal set
--    so the column says so on its own.
ALTER TABLE "provider_tokens" DROP CONSTRAINT IF EXISTS "provider_tokens_scope_chk";
ALTER TABLE "provider_tokens" ADD CONSTRAINT "provider_tokens_scope_chk"
    CHECK ("scope" IN ('global', 'user'));

-- 3) The GitHub PAT pinned to a Workspace. `workspaces.run_as_user_id` (0136)
--    keeps the precedence and names a person instead. The other four providers
--    are static env-var credentials that never reach the git-host seam.
DELETE FROM "workspace_credentials" WHERE "provider" = 'github';

ALTER TABLE "workspace_credentials"
    DROP CONSTRAINT IF EXISTS "workspace_credentials_provider_chk",
    ADD CONSTRAINT "workspace_credentials_provider_chk"
        CHECK ("provider" IN ('gitlab', 'gitness', 'openai', 'anthropic'));

-- 4) Stored repository reachability. It answered "can this repository be
--    reached", which has no answer without naming whose credential asks.
--    Listing branches and creating a Task each refuse by name instead.
ALTER TABLE "repositories"
    DROP COLUMN IF EXISTS "access_status",
    DROP COLUMN IF EXISTS "access_checked_at",
    DROP COLUMN IF EXISTS "access_error";
