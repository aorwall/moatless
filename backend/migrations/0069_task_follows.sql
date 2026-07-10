-- Per-user task follows: a user "follows" a task to keep it in their personal
-- task list. Users auto-follow tasks they create or send a message to, and can
-- explicitly follow/unfollow any task they can see. The sidebar task list is
-- driven by this table rather than by `created_by` alone, so followed tasks
-- created by other users show up alongside a user's own.

CREATE TABLE IF NOT EXISTS "task_follows" (
    "user_id"    text        NOT NULL,
    "task_id"    text        NOT NULL,
    "created_at" timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY ("user_id", "task_id")
);

CREATE INDEX IF NOT EXISTS "task_follows_task_idx"
    ON "task_follows" ("task_id");

CREATE INDEX IF NOT EXISTS "task_follows_user_idx"
    ON "task_follows" ("user_id");

-- Backfill: every creator follows the tasks they created so their existing
-- task list is preserved after the sidebar switches to a follow-based filter.
INSERT INTO "task_follows" ("user_id", "task_id", "created_at")
SELECT t.created_by, t.task_id, COALESCE(t.created_at_db, now())
FROM tasks t
WHERE t.created_by IS NOT NULL
ON CONFLICT DO NOTHING;
