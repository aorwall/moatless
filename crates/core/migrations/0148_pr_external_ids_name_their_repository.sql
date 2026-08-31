-- GitHub PR-level external ids now carry the pull request, not just its
-- number: `github:pr:opened:owner/repo#607` where rows written before this
-- carry `github:pr:opened:607`.
--
-- # Why the old shape could not stay
--
-- GitHub allocates comment, review and run ids once across the host, so those
-- ids name an event by themselves. It numbers pull requests per repository, so
-- `607` does not — it is a pull request in every repository that has one,
-- while `task_messages` and `task_outbound_events` hold every repository's
-- rows together. A reader matching on the id alone therefore matched another
-- repository's pull request, and the GitHub router bound a task to a pull
-- request it had never touched on the strength of it.
--
-- # Why rewrite rather than alias
--
-- The alternative is to teach every reader that the two shapes mean the same
-- event. That hands the ambiguous string back to each of them, which is the
-- collision itself: the point of the format is that a reader cannot ask an
-- ambiguous question. Rewriting leaves one shape in the tables.
--
-- # What identifies each row's repository
--
-- Every row already records where its event happened, so nothing is inferred:
-- `task_messages` carries `external_subject_key` (`owner/repo#607`),
-- `task_outbound_events` carries `source_name` (`owner/repo`, with the number
-- taken from the id being rewritten), and `task_bindings` carries
-- `subject_key`. Where the number appears twice, both must agree before the
-- row is touched.
--
-- A row that records no subject keeps its old id. It cannot be placed, so
-- there is nothing to rewrite it to, and the scoped lookups already decline to
-- answer from it.
--
-- Re-running this changes nothing: the rewritten ids no longer match the
-- number-only pattern each statement selects on.

-- The inbound side: the external event a Message arrived as.
UPDATE "task_messages"
   SET "external_event_id" = regexp_replace(
           "external_event_id",
           '^github:(pr:[a-z_]+|conflict):[0-9]+',
           'github:\1:' || "external_subject_key"
       )
 WHERE "external_event_id" ~ '^github:(pr:[a-z_]+|conflict):[0-9]+(:|$)'
   AND "external_subject_key" ~ '^[^/#:]+/[^/#:]+#[0-9]+$'
   AND substring("external_event_id" from '^github:(?:pr:[a-z_]+|conflict):([0-9]+)')
       = substring("external_subject_key" from '#([0-9]+)$');

-- The outbound side: the external events a Task caused. `external_event_id` is
-- the primary key here, so the rewrite is guarded against landing on an id
-- that already exists rather than aborting the migration on a duplicate. The
-- mapping is injective — same action, same source, distinct numbers — so the
-- guard only ever fires on a re-run against already-rewritten data.
WITH "remapped" AS (
    SELECT "external_event_id" AS "old_id",
           regexp_replace(
               "external_event_id",
               '^github:(pr:[a-z_]+|conflict):([0-9]+)',
               'github:\1:' || "source_name" || '#\2'
           ) AS "new_id"
      FROM "task_outbound_events"
     WHERE "external_event_id" ~ '^github:(pr:[a-z_]+|conflict):[0-9]+(:|$)'
       AND "source_name" ~ '^[^/#:]+/[^/#:]+$'
)
UPDATE "task_outbound_events" AS "e"
   SET "external_event_id" = "r"."new_id"
  FROM "remapped" AS "r"
 WHERE "e"."external_event_id" = "r"."old_id"
   AND NOT EXISTS (
       SELECT 1
         FROM "task_outbound_events" AS "x"
        WHERE "x"."external_event_id" = "r"."new_id"
   );

-- The delivery cursor a Binding keeps, so it keeps naming the event it means.
UPDATE "task_bindings"
   SET "last_external_event_id" = regexp_replace(
           "last_external_event_id",
           '^github:(pr:[a-z_]+|conflict):[0-9]+',
           'github:\1:' || "subject_key"
       )
 WHERE "subject_kind" = 'github_pr'
   AND "last_external_event_id" ~ '^github:(pr:[a-z_]+|conflict):[0-9]+(:|$)'
   AND "subject_key" ~ '^[^/#:]+/[^/#:]+#[0-9]+$'
   AND substring("last_external_event_id" from '^github:(?:pr:[a-z_]+|conflict):([0-9]+)')
       = substring("subject_key" from '#([0-9]+)$');
