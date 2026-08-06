-- Every Loop has a name.
--
-- `triggers.name` has been nullable since the table was introduced, so a Loop
-- could exist with nothing to call it — the UI fell back to a cron expression
-- or a channel id, and a Git-declared Loop borrowed its directory name.
--
-- Naming a Loop is an editorial act, so it happens through the API
-- (`moat loops update --id <id> --name "..."`), not by deriving something from
-- a matcher here. The three unnamed Loops on the cluster were named that way
-- before this migration. The `name = id` below is only the floor that lets the
-- column go NOT NULL on a database nobody got to first: it cannot fail, and an
-- id sitting in the name column reads unmistakably as "still unnamed".

BEGIN;

UPDATE "triggers"
SET "name" = "id",
    "updated_at" = now()
WHERE "name" IS NULL OR btrim("name") = '';

ALTER TABLE "triggers"
    ALTER COLUMN "name" SET NOT NULL;

-- Dropped first so the statement also lands on a database that already carries
-- the constraint from an out-of-band apply.
ALTER TABLE "triggers"
    DROP CONSTRAINT IF EXISTS "triggers_name_not_blank_check";

ALTER TABLE "triggers"
    ADD CONSTRAINT "triggers_name_not_blank_check"
    CHECK (btrim("name") <> '');

COMMIT;
