-- A Workspace's own say in which plugin skills load.
--
-- `plugin_activations` gains a third subject. A row names at most one of them:
-- `user_id` set is one person's record, `workspace_id` set is one Workspace's,
-- neither set is the deployment-wide default. A row naming both would have no
-- place in the precedence order, so the CHECK refuses it rather than leaving
-- resolution to pick.

ALTER TABLE "plugin_activations"
    ADD COLUMN IF NOT EXISTS "workspace_id" text
        REFERENCES "workspaces"("id") ON DELETE CASCADE;

ALTER TABLE "plugin_activations"
    DROP CONSTRAINT IF EXISTS "plugin_activations_one_subject";
ALTER TABLE "plugin_activations"
    ADD CONSTRAINT "plugin_activations_one_subject"
        CHECK ("user_id" IS NULL OR "workspace_id" IS NULL);

-- The unique key has to carry the new column, or a Workspace's record would
-- collide with the global one it exists to override.
ALTER TABLE "plugin_activations"
    DROP CONSTRAINT IF EXISTS "plugin_activations_plugin_id_skill_name_user_id_key";
ALTER TABLE "plugin_activations"
    ADD CONSTRAINT "plugin_activations_subject_key"
        UNIQUE NULLS NOT DISTINCT ("plugin_id", "skill_name", "user_id", "workspace_id");

-- The global index meant "the records that apply to everybody". With a third
-- subject in the table, `user_id IS NULL` alone now also matches every
-- Workspace record, so it has to name both columns to keep meaning that.
DROP INDEX IF EXISTS "idx_plugin_activations_global";
CREATE INDEX IF NOT EXISTS "idx_plugin_activations_global"
    ON "plugin_activations"("plugin_id")
    WHERE "user_id" IS NULL AND "workspace_id" IS NULL;

-- Resolution reads one Workspace's records alongside one person's, so that is
-- the third read shape worth an index.
CREATE INDEX IF NOT EXISTS "idx_plugin_activations_workspace"
    ON "plugin_activations"("workspace_id") WHERE "workspace_id" IS NOT NULL;
