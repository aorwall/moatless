-- Where an activity sits *inside* its change.
--
-- Spec: `.plans/t3-thread-view-surfaces.md`. `position` is the change position,
-- allocated once from `task_conversation_position` and written to every activity
-- of that change (`0118_task_activities.sql:23-25`), so every activity of one
-- turn carries the same one. `ordinal` is what separates them, and the pair
-- `(position, ordinal)` is the order a conversation is read and numbered in.
--
-- The value is the block's own index, read off the source block rather than
-- counted while mapping, so it is the same number on every pass. That is what
-- makes it stable for exactly as long as `activity_id` is, and why redelivery
-- leaves it alone: `ON CONFLICT DO UPDATE` excludes it beside `position`.
--
-- `0` for a Sandbox activity and for every other row that has no block: each is
-- alone in its change.
--
-- Default 0 rather than a backfill. A conversation written before this column
-- existed keeps rendering in the order it already had — every one of its
-- activities lands in the same slot, and `activity_id` breaks the tie.
ALTER TABLE task_activities
    ADD COLUMN IF NOT EXISTS ordinal integer NOT NULL DEFAULT 0;

COMMENT ON COLUMN task_activities.ordinal IS
    'The activity''s place inside its change: the source block''s index, or 0 for a row with no block. Ordered by (position, ordinal); never moved by a redelivery.';

-- The closed set `0118_task_activities.sql:32-36` describes grows here, and
-- stays closed: `tool_call`, `thinking`, `sandbox_status`, `sandbox_error`,
-- `user_input_requested`, `user_input_resolved`, `plan_updated`,
-- `context_window_updated`. Still `text` with a comment rather than an enum
-- type, for the reason written there — adding a value must be a code change
-- that fails to compile, not a migration. The comment is restated here because
-- `sqlx::migrate!` checksums what it has already applied, so an applied file
-- stays byte-for-byte as it was.
COMMENT ON COLUMN task_activities.kind IS
    'Closed set: tool_call, thinking, sandbox_status, sandbox_error, user_input_requested, user_input_resolved, plan_updated, context_window_updated. A value outside it is a bug, not an extension point.';
