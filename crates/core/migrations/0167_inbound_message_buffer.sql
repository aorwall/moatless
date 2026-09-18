-- Hold inbound Adapter events for one Binding briefly, so a burst arriving
-- together reaches the Task as one message instead of one message per event.
--
-- A GitHub review with twelve comments fires twelve webhooks inside a second,
-- and each one became its own Task turn. Buffering moves the durable seam here:
-- routing commits the row and the delivery record succeeds, so a restart
-- between admission and flush loses nothing.
CREATE TABLE IF NOT EXISTS "inbound_message_buffer" (
    "id" text PRIMARY KEY NOT NULL,
    -- Total order within a binding. The combined message lists parts oldest
    -- first, and two events sharing a `created_at` still order.
    "seq" bigserial NOT NULL,
    "binding_id" text NOT NULL REFERENCES "task_bindings"("id") ON DELETE CASCADE,
    "loop_id" text NOT NULL,
    "adapter_label" text NOT NULL,
    -- The normalized `InboundMessage`, stored whole and composed at flush time
    -- rather than at admission, so one thread-context fetch serves the batch.
    "message" jsonb NOT NULL,
    -- When the window opened by the first buffered event closes. Fixed at
    -- admission, so a steady stream of events cannot starve the flush.
    "flush_at" timestamptz NOT NULL,
    "attempts" integer NOT NULL DEFAULT 0,
    "claimed_at" timestamptz,
    "claimed_by" text,
    "created_at" timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS "inbound_message_buffer_due_idx"
    ON "inbound_message_buffer" ("flush_at") WHERE "claimed_at" IS NULL;
CREATE INDEX IF NOT EXISTS "inbound_message_buffer_binding_idx"
    ON "inbound_message_buffer" ("binding_id", "seq");

COMMENT ON TABLE "inbound_message_buffer" IS
    'Inbound adapter events held for their binding''s buffer window, flushed as one combined task message.';

-- Seconds this Loop holds inbound events before combining them. NULL falls back
-- to the environment default for the Loop's adapter kind; 0 turns buffering off
-- and delivers every event on its own.
ALTER TABLE loop_configs ADD COLUMN IF NOT EXISTS message_buffer_seconds integer;
