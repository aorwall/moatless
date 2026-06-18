-- Distinguish triggers synced from a repo's committed `.moatless/loops/`
-- ("loops") from manually-created triggers so reconciliation never touches
-- the manual ones.
--
--   managed_by = NULL        -> manual / API-created trigger (default)
--   managed_by = 'repo_loop' -> synced from `.moatless/loops/<loop_name>/LOOP.md`
--
-- `(source_repository_id, loop_name)` is the stable upsert key for synced loops.

BEGIN;

ALTER TABLE "triggers"
    ADD COLUMN IF NOT EXISTS "managed_by" text,
    ADD COLUMN IF NOT EXISTS "source_repository_id" text,
    ADD COLUMN IF NOT EXISTS "loop_name" text;

-- One synced trigger per (repo, loop name). Scoped to repo_loop rows so it
-- never collides with manual triggers (which leave these columns NULL).
CREATE UNIQUE INDEX IF NOT EXISTS "triggers_loop_idx"
    ON "triggers" ("source_repository_id", "loop_name")
    WHERE "managed_by" = 'repo_loop';

-- Lookup all synced loops for a repo during reconcile.
CREATE INDEX IF NOT EXISTS "triggers_source_repository_idx"
    ON "triggers" ("source_repository_id");

COMMIT;
