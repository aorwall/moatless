-- One word for plugins: `plugin_repos` becomes `plugins`.
--
-- The stored data called it a plugin repo, the API called it a plugin repo,
-- the frontend called it a marketplace and the CLI called it a plugin. This
-- renames the stored side so the vocabulary matches everywhere.
--
-- The repository link is renamed here and removed later, once activation
-- records replace it — it is currently the only way a plugin reaches a
-- sandbox.

ALTER TABLE "plugin_repos" RENAME TO "plugins";
ALTER TABLE "repository_plugin_repos" RENAME TO "repository_plugins";
ALTER TABLE "repository_plugins" RENAME COLUMN "plugin_repo_id" TO "plugin_id";

ALTER INDEX IF EXISTS "idx_repo_plugin_repos_repo" RENAME TO "idx_repository_plugins_repository";

-- The never-completed second delivery mechanism goes with it. A plugin
-- selection stored on a workspace fed an identifier format resolved by a
-- download endpoint that was never built, so no selection ever reached a
-- sandbox and no stored value here has meaning.

ALTER TABLE "workspaces" DROP COLUMN IF EXISTS "plugins";
ALTER TABLE "repositories" DROP COLUMN IF EXISTS "plugins";
