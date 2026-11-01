-- A `secret_requested` activity, the secret card an agent shows in its thread,
-- and the `secret_request_answered` activity that settles it. Both are
-- projected from task events of the same names and carry no secret value.
ALTER TABLE conversation_activities DROP CONSTRAINT IF EXISTS conversation_activities_kind_check;
ALTER TABLE conversation_activities ADD CONSTRAINT conversation_activities_kind_check CHECK (kind IN (
    'tool_call', 'thinking', 'user_input_requested', 'plan_updated',
    'user_input_resolved', 'sandbox_status', 'sandbox_warning',
    'sandbox_error', 'context_window_updated', 'agent_switched', 'compaction',
    'artifact_shown', 'secret_requested', 'secret_request_answered'));
