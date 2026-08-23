-- One task per subject, enforced across backend replicas.
--
-- `route_inbound_message` decided whether to create a task by reading the
-- bindings for a subject and testing them for emptiness. That read and the
-- create that follows are not atomic, so two events for the same subject
-- arriving on different replicas both saw no binding and both created a task.
--
-- It happens in ordinary use: an @mention reaches the Slack adapter as two
-- events carrying the same message ts, roughly 300ms apart. The event-receipt
-- table could not collapse them because its key includes `task_id`, and each
-- event had just made a different one.
--
-- This table is the claim that makes creation single-winner. The insert is the
-- decision: whoever lands the row creates the task and records it here;
-- everyone else is told to retry, by which time the binding exists.
CREATE TABLE IF NOT EXISTS "adapter_task_creation_claims" (
    "loop_id" text NOT NULL,
    "subject_kind" text NOT NULL,
    "subject_key" text NOT NULL,
    "adapter_kind" text NOT NULL,
    "external_event_id" text,
    "task_id" text,
    "claimed_at" timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY ("loop_id", "subject_kind", "subject_key")
);

COMMENT ON TABLE "adapter_task_creation_claims" IS
    'Single-winner claim for creating a task from an inbound adapter event. One row per (loop, subject); the winner records its task_id, losers retry and deliver to it.';

-- Backfill from the bindings that already exist, so no claim is handed out for
-- a subject that has had a task since before this table. `ingress` is the one
-- origin the router writes from an inbound event; every other origin was
-- placed by a task that already existed and must not read as this loop having
-- created one.
INSERT INTO "adapter_task_creation_claims" (
    "loop_id", "subject_kind", "subject_key",
    "adapter_kind", "task_id", "claimed_at"
)
SELECT DISTINCT ON (b."loop_id", b."subject_kind", b."subject_key")
    b."loop_id", b."subject_kind", b."subject_key",
    b."adapter_kind", b."task_id", b."created_at"
FROM "task_bindings" b
WHERE b."loop_id" IS NOT NULL
  AND b."origin" = 'ingress'
ORDER BY b."loop_id", b."subject_kind", b."subject_key", b."created_at"
ON CONFLICT DO NOTHING;
