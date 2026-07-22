-- =========================================================================
-- Workspace git provenance — source config path
-- =========================================================================
-- `source_config_path` records which file in the source repository declared
-- this workspace (`.moatless/workspaces.json` or `.moatless/config.json`).
-- Together with `source_repository_id` and `source_key` it gives a full
-- provenance trail even after the workspace is overridden (source='manual').
--
-- Overriding a git-synced workspace sets source='manual' while keeping these
-- provenance columns intact so the origin is always visible. The git sync
-- service skips workspaces that have source='manual' — they won't be updated
-- or reconcile-deleted until explicitly reset back to source='git'.

ALTER TABLE "workspaces"
  ADD COLUMN IF NOT EXISTS "source_config_path" text;
