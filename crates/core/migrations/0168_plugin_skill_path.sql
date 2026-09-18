-- Where a plugin skill is edited.
--
-- A skill listing named only the skill. Every copy an agent can reach on a pod
-- is regenerated on init and dies with it, so an agent asked to change one
-- edited the copy and reached nobody. Record the SKILL.md path inside the
-- plugin repository, which is the copy an edit has to land in.
--
-- Backfilled empty: the next sync of each plugin rewrites the whole skill set.

ALTER TABLE "plugin_skills" ADD COLUMN IF NOT EXISTS "path" text NOT NULL DEFAULT '';
