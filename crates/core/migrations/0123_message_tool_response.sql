-- A person's answer, as an answer.
--
-- Spec: `.plans/t3-thread-view-surfaces.md`. A `tool_response` row exists to
-- hand a person's answer back to the agent, and the answer leaves it in two
-- shapes because two paths read it. The structure lives here: an answer per
-- question, or a plan verdict, which is what the sandbox delivers to a run
-- still parked on the call. `message_text` keeps the prose — the question
-- restated with the answer — which is what reaches an agent asked again in a
-- fresh turn, the sandbox having been reclaimed while the question stood.
--
-- Shape: `{toolUseId, toolName, answers?, approved?, feedback?}`. `answers` is
-- keyed on each question's full text, because that is the key the CLI looks an
-- answer up by. NULL on every row that is not a tool response, which is nearly
-- all of them.
ALTER TABLE task_messages
    ADD COLUMN IF NOT EXISTS tool_response jsonb;

COMMENT ON COLUMN task_messages.tool_response IS
    'A person''s answer to an interactive tool call: {toolUseId, toolName, answers?, approved?, feedback?}. NULL unless this row is a tool response.';
