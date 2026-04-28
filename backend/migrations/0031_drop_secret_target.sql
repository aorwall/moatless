-- Drop target-based routing from secrets and audits.
-- Secret delivery is now flat: all accessible secrets go to every consumer
-- in the task (agent env + server env file). Routing by `target` is gone.

ALTER TABLE "secrets" DROP CONSTRAINT IF EXISTS "secrets_target_chk";
DROP INDEX IF EXISTS "secrets_scope_key_target_uq";

-- Collapse any duplicates that existed across targets before dropping the column.
-- Keep the newest row per (scope_type, scope_id, key).
DELETE FROM "secrets" s
USING "secrets" s2
WHERE s.scope_type = s2.scope_type
  AND COALESCE(s.scope_id, '') = COALESCE(s2.scope_id, '')
  AND s.key = s2.key
  AND (s.updated_at, s.id) < (s2.updated_at, s2.id);

ALTER TABLE "secrets" DROP COLUMN "target";
ALTER TABLE "secret_audit_events" DROP COLUMN "target";

CREATE UNIQUE INDEX IF NOT EXISTS "secrets_scope_key_uq"
    ON "secrets" ("scope_type", COALESCE("scope_id", ''), "key");
