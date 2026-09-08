-- Every Loop names the Workspace its tasks run in, and nothing else.
--
-- The backfill picks what a firing already picked for a Loop that named none:
-- the live Workspaces placing `repository_id`, primary placement first, then
-- oldest first. A Loop whose repository places no live Workspace could not open
-- a task at all, so it is deleted rather than left holding a NULL.
--
-- Deleting a `loops` row cascades its config, subscription, schedule and
-- pairing requests, and nulls `tasks.loop_id` and `task_bindings.loop_id`:
-- historical tasks lose their Loop irreversibly. Count them before running this.

UPDATE "loop_configs" c
SET "workspace_id" = (
    SELECT w."id"
      FROM "workspace_repos" r
      JOIN "workspaces" w ON w."id" = r."workspace_id"
     WHERE r."repository_id" = c."repository_id"
       AND (w."deleted" IS NULL OR w."deleted" = false)
     GROUP BY w."id"
     ORDER BY bool_or(r."is_primary") DESC, w."created_at" ASC, w."id" ASC
     LIMIT 1
)
WHERE c."workspace_id" IS NULL;

DELETE FROM "loops"
 WHERE "id" IN (SELECT "loop_id" FROM "loop_configs" WHERE "workspace_id" IS NULL);

ALTER TABLE "loop_configs"
    ALTER COLUMN "workspace_id" SET NOT NULL;

ALTER TABLE "loop_configs"
    DROP COLUMN "repository_id";
