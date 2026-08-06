-- Per-user read receipts: track the last time a user viewed a task's
-- conversation. A task is "unread" for a user when its last_message_at is
-- newer than that user's last_read_at (or no receipt exists yet).

CREATE TABLE IF NOT EXISTS "task_read_receipts" (
    "user_id"      text        NOT NULL,
    "task_id"      text        NOT NULL,
    "last_read_at" timestamptz NOT NULL,
    PRIMARY KEY ("user_id", "task_id")
);

CREATE INDEX IF NOT EXISTS "task_read_receipts_task_idx"
    ON "task_read_receipts" ("task_id");
