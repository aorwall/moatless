-- An `agent_switched` activity: the platform moved a Task to another agent.
ALTER TABLE conversation_activities DROP CONSTRAINT IF EXISTS conversation_activities_kind_check;
ALTER TABLE conversation_activities ADD CONSTRAINT conversation_activities_kind_check CHECK (kind IN (
    'tool_call', 'thinking', 'user_input_requested', 'plan_updated',
    'user_input_resolved', 'sandbox_status', 'sandbox_warning',
    'sandbox_error', 'context_window_updated', 'agent_switched'));
