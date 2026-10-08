-- A `compaction` activity: the harness compacted its context. It is projected
-- from the Trajectory line carrying the summary, so the source check admits it
-- beside the other line-sourced kinds.
ALTER TABLE conversation_activities DROP CONSTRAINT IF EXISTS conversation_activities_kind_check;
ALTER TABLE conversation_activities ADD CONSTRAINT conversation_activities_kind_check CHECK (kind IN (
    'tool_call', 'thinking', 'user_input_requested', 'plan_updated',
    'user_input_resolved', 'sandbox_status', 'sandbox_warning',
    'sandbox_error', 'context_window_updated', 'agent_switched', 'compaction'));

-- 0178 left the source check unnamed, so it is found by its definition.
DO $$
DECLARE
    source_check text;
BEGIN
    SELECT conname INTO source_check
      FROM pg_constraint
     WHERE conrelid = 'conversation_activities'::regclass
       AND contype = 'c'
       AND pg_get_constraintdef(oid) LIKE '%user_input_resolved%'
       AND pg_get_constraintdef(oid) LIKE '%task_event_id IS NOT NULL%';
    IF source_check IS NOT NULL THEN
        EXECUTE format('ALTER TABLE conversation_activities DROP CONSTRAINT %I', source_check);
    END IF;
END $$;

ALTER TABLE conversation_activities ADD CONSTRAINT conversation_activities_source_check CHECK (frozen OR CASE
  WHEN kind = 'tool_call' THEN trajectory_event_id IS NOT NULL AND tool_use_id IS NOT NULL
                               AND num_nonnulls(inbox_id, task_event_id) = 0
  WHEN kind IN ('thinking', 'user_input_requested', 'plan_updated', 'compaction')
                          THEN trajectory_event_id IS NOT NULL
                               AND num_nonnulls(tool_use_id, result_event_id, inbox_id,
                                                task_event_id) = 0
  WHEN kind = 'user_input_resolved'
                          THEN inbox_id IS NOT NULL
                               AND num_nonnulls(trajectory_event_id, result_event_id,
                                                tool_use_id, task_event_id) = 0
  ELSE task_event_id IS NOT NULL
       AND num_nonnulls(trajectory_event_id, result_event_id, tool_use_id, inbox_id) = 0
END);
