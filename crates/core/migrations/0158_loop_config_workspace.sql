-- The Workspace the tasks a Loop spawns run in.
--
-- NULL falls back to a live Workspace placing `repository_id`, primary
-- placement first, so a Loop written before this column keeps firing. Declared
-- in `.moatless/loops/*/LOOP.md` as `workspace:` and resolved to an id at sync.

ALTER TABLE "loop_configs"
    ADD COLUMN IF NOT EXISTS "workspace_id" text;
