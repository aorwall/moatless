-- An external event a Task caused is a fact about the Task, not about the tool
-- call that made it. `task_outbound_events` holds every such record, and these
-- columns held a second, differently keyed copy for the calls that had a
-- `tool_use_id`.
--
-- Carry the existing rows across so a Task running through this migration keeps
-- suppressing the echoes of what it already posted. The join to `tasks` is
-- required: `task_activities` has no foreign key on `task_id`, because a record
-- must outlive what it names, while an outbound event naming a Task nothing can
-- deliver to is useless.
INSERT INTO "task_outbound_events" ("external_event_id", "task_id", "provider", "source_name", "created_at")
SELECT DISTINCT ON (a."external_event_id")
       a."external_event_id",
       a."task_id",
       coalesce(a."external_adapter", 'unknown'),
       a."external_source_name",
       a."created_at"
  FROM "task_activities" a
  JOIN "tasks" t ON t."task_id" = a."task_id"
 WHERE a."external_event_id" IS NOT NULL
 ORDER BY a."external_event_id", a."created_at" DESC
    ON CONFLICT ("external_event_id") DO NOTHING;

ALTER TABLE "task_activities"
    DROP COLUMN IF EXISTS "external_event_id",
    DROP COLUMN IF EXISTS "external_adapter",
    DROP COLUMN IF EXISTS "external_source_name";
