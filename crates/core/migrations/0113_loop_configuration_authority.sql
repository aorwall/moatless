-- Separate Loop configuration authority from execution approval.
--
-- Git owns configuration only. Execution always requires a later explicit
-- administrator activation with a concrete run-as user.

BEGIN;

DROP INDEX IF EXISTS "triggers_loop_idx";
DROP INDEX IF EXISTS "triggers_source_repository_idx";

ALTER TABLE "triggers"
    ADD COLUMN IF NOT EXISTS "config_source" text,
    ADD COLUMN IF NOT EXISTS "source_config_path" text,
    ADD COLUMN IF NOT EXISTS "deleted" boolean NOT NULL DEFAULT false;

UPDATE "triggers"
SET "config_source" = CASE
    WHEN "managed_by" = 'repo_loop' THEN 'git'
    ELSE 'manual'
END
WHERE "config_source" IS NULL;

ALTER TABLE "triggers"
    ALTER COLUMN "config_source" SET DEFAULT 'manual',
    ALTER COLUMN "config_source" SET NOT NULL;

ALTER TABLE "triggers"
    ADD CONSTRAINT "triggers_config_source_check"
    CHECK ("config_source" IN ('git', 'manual'));

ALTER TABLE "triggers"
    RENAME COLUMN "loop_name" TO "source_key";

UPDATE "triggers"
SET "source_config_path" = '.moatless/loops/' || "source_key" || '/LOOP.md'
WHERE "source_repository_id" IS NOT NULL
  AND "source_key" IS NOT NULL
  AND "source_config_path" IS NULL;

-- Existing Git Loops have never received the new administrator approval.
UPDATE "triggers"
SET "active" = false,
    "updated_at" = now()
WHERE "config_source" = 'git';

UPDATE "trigger_rules" r
SET "run_as_user_id" = NULL
FROM "triggers" t
WHERE r."trigger_id" = t."id"
  AND t."config_source" = 'git';

-- No Loop may remain runnable through the legacy created-by fallback.
UPDATE "triggers" t
SET "active" = false,
    "updated_at" = now()
WHERE t."active" = true
  AND NOT EXISTS (
      SELECT 1
      FROM "trigger_rules" r
      WHERE r."trigger_id" = t."id"
        AND r."run_as_user_id" IS NOT NULL
  );

ALTER TABLE "triggers" DROP COLUMN "managed_by";

-- Provenance remains unique after override and deletion so a later Git sync
-- skips the local row instead of creating a duplicate.
CREATE UNIQUE INDEX "triggers_git_provenance_idx"
    ON "triggers" ("source_repository_id", "source_key")
    WHERE "source_repository_id" IS NOT NULL AND "source_key" IS NOT NULL;

CREATE INDEX "triggers_source_repository_idx"
    ON "triggers" ("source_repository_id");

COMMIT;
