-- Turn-level agent metrics from the SDK result message.
--
-- Per-message token counts are unreliable for Claude Code: its stdout stream
-- reports a partial `output_tokens` on each assistant line, while the final
-- count only reaches the transcript. The terminal result message carries the
-- authoritative turn totals plus per-model cost, so they are stored here —
-- on the turn they describe rather than smeared onto one message row.
ALTER TABLE "task_turns" ADD COLUMN IF NOT EXISTS "metrics" jsonb;
