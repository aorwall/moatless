-- Reasoning effort, per turn.
--
-- A sibling of `model` and `agent_mode`: what the person asked the agent to
-- spend on this turn, in the agent's own vocabulary ("low"/"medium"/"high"/
-- "xhigh"/"max" for Claude, "minimal"/"low"/"medium"/"high" for Codex). Free
-- text for the same reason `agent_mode` is: the value is consumed by the
-- Sandbox and the agent CLI behind it, and neither the backend nor the database
-- is the authority on which levels an agent offers.
--
-- NULL means the person expressed no preference, and the agent runs on its own
-- default. That is what nearly every row says.
ALTER TABLE task_messages
    ADD COLUMN IF NOT EXISTS effort text;

COMMENT ON COLUMN task_messages.effort IS
    'Reasoning effort for the turn, in the agent''s own vocabulary. NULL means the agent''s own default.';
