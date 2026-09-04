-- Per-loop model override for the tasks a Loop spawns.
--
-- `agent_type` already picks the agent family (claude-code, codex); `model`
-- pins the LLM within it (e.g. `opus`). NULL keeps the platform default the
-- sandbox resolves from `DEFAULT_CLAUDE_CODE_MODEL`, so an absent value changes
-- nothing. The spawn path reads it into `NewTaskRequest.model`.

ALTER TABLE "loop_configs"
    ADD COLUMN IF NOT EXISTS "model" text;
