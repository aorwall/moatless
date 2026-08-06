-- Collapse per_message and thread_based into by_subject. Both modes were
-- behaviorally identical at runtime; the distinction was documentation only.
UPDATE "task_routes"
   SET "routing_mode" = 'by_subject'
 WHERE "routing_mode" IN ('per_message', 'thread_based');

ALTER TABLE "task_routes"
  ALTER COLUMN "routing_mode" SET DEFAULT 'by_subject';
