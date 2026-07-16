-- assistant_usage: full raw token-usage object from an assistant message.
--
-- The four assistant_usage_*_tokens columns capture only the headline counts.
-- The Claude usage block additionally carries the ephemeral cache-write
-- breakdown (ephemeral_1h vs ephemeral_5m — priced differently) and
-- server_tool_use counts, which are needed to derive cost. The tmux transport
-- never reports total_cost_usd (the TUI transcript has no cost field), so
-- persisting the complete usage here lets cost be computed downstream from
-- tokens + model for every transport. Populated for assistant rows only.
ALTER TABLE task_messages ADD COLUMN IF NOT EXISTS assistant_usage jsonb;
