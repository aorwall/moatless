-- =========================================================================
-- Git-synced workspaces — source provenance columns
-- =========================================================================
-- A workspace can be declared in a repo's git tree (`.moatless/workspaces.json`
-- or the legacy `.moatless/config.json`) and reconciled into a `workspaces` row
-- flagged `source = 'git'`. Git-synced workspaces are read-only in the UI and
-- write API — only a re-sync from git changes them — and are keyed by
-- `(source_repository_id, source_key)` so reconciliation never touches
-- manually-created workspaces.
--
-- `network_profile` is added for parity with `repositories`: the Phase A backfill
-- copied 9 run-config columns but omitted it, so without this column
-- provisioning-from-workspace would silently drop the network profile.

ALTER TABLE "workspaces"
  ADD COLUMN IF NOT EXISTS "source" text NOT NULL DEFAULT 'manual',  -- 'manual' | 'git'
  ADD COLUMN IF NOT EXISTS "source_repository_id" text,
  ADD COLUMN IF NOT EXISTS "source_key" text,
  ADD COLUMN IF NOT EXISTS "network_profile" text;                   -- parity with repositories

CREATE INDEX IF NOT EXISTS "workspaces_source_repository_idx"
  ON "workspaces" ("source_repository_id") WHERE "source_repository_id" IS NOT NULL;

-- One git-synced workspace per (source repo, source key). No FK on
-- `source_repository_id` — consistent with `workspace_repos.repository_id` and
-- `tasks.workspace_id`, both intentionally FK-free during the transition.
CREATE UNIQUE INDEX IF NOT EXISTS "workspaces_git_source_unique"
  ON "workspaces" ("source_repository_id", "source_key") WHERE "source" = 'git';
