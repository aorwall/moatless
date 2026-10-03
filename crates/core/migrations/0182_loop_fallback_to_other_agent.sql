-- Re-run a Loop task's turn on the other harness (codex <-> claude-code) when
-- its provider will not serve it. On by default, existing Loops included.
ALTER TABLE loop_configs
    ADD COLUMN IF NOT EXISTS fallback_to_other_agent boolean NOT NULL DEFAULT true;
