-- Feedback table for capturing qualitative feedback on agent runs.
--
-- Feedback can be scoped at three levels:
--   * task-level (message_uuid IS NULL, tool_use_id IS NULL)
--   * message-level (message_uuid set, tool_use_id IS NULL)
--   * tool-call-level (both set; tool_use_id requires message_uuid)
--
-- `source` discriminates between human and agent submissions, and `surface`
-- records where in the UI/CLI the feedback originated.

CREATE TABLE IF NOT EXISTS feedback (
    id            bigserial    PRIMARY KEY,
    feedback_id   text         NOT NULL UNIQUE,
    task_id       text         NOT NULL,
    message_uuid  text,
    tool_use_id   text,
    source        text         NOT NULL,
    surface       text         NOT NULL,
    comment       text         NOT NULL,
    metadata      jsonb,
    created_by    text         NOT NULL,
    created_at    timestamptz  NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS feedback_task_id_idx ON feedback (task_id);
CREATE INDEX IF NOT EXISTS feedback_message_uuid_idx
    ON feedback (message_uuid) WHERE message_uuid IS NOT NULL;
CREATE INDEX IF NOT EXISTS feedback_tool_use_id_idx
    ON feedback (tool_use_id) WHERE tool_use_id IS NOT NULL;
