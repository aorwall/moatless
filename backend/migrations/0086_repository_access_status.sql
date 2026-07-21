-- =========================================================================
-- Repository access status — fail state for auto-created repos
-- =========================================================================
-- When a workspace declared in git references a repository URL, the sync
-- find-or-creates a `repositories` row for it and verifies reachability against
-- the provider. Missing or inaccessible repos still get a row and a placement,
-- flagged here so the UI can surface the problem instead of silently dropping it.

ALTER TABLE "repositories"
  ADD COLUMN IF NOT EXISTS "access_status" text NOT NULL DEFAULT 'unchecked', -- unchecked|ok|not_found|forbidden|error
  ADD COLUMN IF NOT EXISTS "access_checked_at" timestamptz,
  ADD COLUMN IF NOT EXISTS "access_error" text;
