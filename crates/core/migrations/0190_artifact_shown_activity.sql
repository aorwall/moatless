-- An `artifact_shown` activity: an Artifact version published with `--inline`,
-- shown in the thread as the page itself. It is projected from an
-- `artifact_shown` task event, so the source check's task-event branch admits
-- it unchanged.
ALTER TABLE conversation_activities DROP CONSTRAINT IF EXISTS conversation_activities_kind_check;
ALTER TABLE conversation_activities ADD CONSTRAINT conversation_activities_kind_check CHECK (kind IN (
    'tool_call', 'thinking', 'user_input_requested', 'plan_updated',
    'user_input_resolved', 'sandbox_status', 'sandbox_warning',
    'sandbox_error', 'context_window_updated', 'agent_switched', 'compaction',
    'artifact_shown'));

-- The versions published with `--inline`. Their page is served with T3's
-- theme and height bootstrap injected, wherever it is opened.
ALTER TABLE task_artifacts ADD COLUMN IF NOT EXISTS inline_versions integer[] NOT NULL DEFAULT '{}';
