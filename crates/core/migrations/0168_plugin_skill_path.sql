-- Where a plugin skill is edited: the SKILL.md path inside the plugin
-- repository. `SkillResponse::path` holds the contract.
--
-- Backfilled empty: the next sync of each plugin rewrites the whole skill set.

ALTER TABLE "plugin_skills" ADD COLUMN IF NOT EXISTS "path" text NOT NULL DEFAULT '';
