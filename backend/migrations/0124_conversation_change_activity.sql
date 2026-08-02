-- What a change is about, when it is about an activity rather than a Message.
--
-- `message_uuid` names a Message; this names an activity that happened on no
-- Message — a Sandbox lifecycle change, a context-window reading. At most one
-- of the two is set; neither set means the conversation's own session moved
-- (its Turn, its Sandbox intent) and nothing else.
--
-- A name, not a description, for the same reason `message_uuid` is one: the
-- reader re-reads the row at delivery time and sends what it finds. And a name
-- rather than the position it was written at, for the reason the Message path
-- learned: `settle` updates a row in place and leaves it at the position of the
-- call, so the change that records a result is at a later position than the row
-- it means. Position says when; only a name says what.
--
-- No index: this is `task_activities`' primary key, and the reader looks up by
-- it. No foreign key, as on `message_uuid` and for the same reason — a record
-- must outlive what it names.
ALTER TABLE task_conversation_changes ADD COLUMN IF NOT EXISTS activity_id text;

-- The closed set `0122_activity_ordinal.sql:34-35` describes grows here, and
-- stays closed: `sandbox_warning` joins `sandbox_status` and `sandbox_error` as
-- the third Sandbox severity — the stall a run recovers from. Still `text` with
-- a comment rather than an enum type, for the reason written there: adding a
-- value must be a code change that fails to compile, not a migration. Restated
-- here rather than edited there because `sqlx::migrate!` checksums an applied
-- file, so it stays byte-for-byte as it was.
COMMENT ON COLUMN task_activities.kind IS
    'Closed set: tool_call, thinking, sandbox_status, sandbox_warning, sandbox_error, user_input_requested, user_input_resolved, plan_updated, context_window_updated. A value outside it is a bug, not an extension point.';
