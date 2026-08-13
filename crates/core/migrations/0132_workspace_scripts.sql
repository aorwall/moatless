-- Workspace scripts — named commands run on demand in the sandbox.
--
-- Beside "servers", and with the same '[]' default so a pre-existing workspace
-- reads as "no scripts" rather than as null. No backfill: deriving scripts from
-- a workspace's servers is exactly the mistake this column replaces.
-- -------------------------------------------------------------------------
ALTER TABLE "workspaces" ADD COLUMN IF NOT EXISTS "scripts" jsonb DEFAULT '[]'::jsonb;
