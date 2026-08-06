-- Partial index supporting the background Codex refresher sweep. The sweeper
-- selects all healthy codex rows (`provider = 'codex'` and no
-- `metadata->>'refresh_failure'`) so it can proactively refresh them before
-- the upstream OAuth refresh_token rotates. The partial predicate keeps the
-- index tiny (only healthy codex rows) and lets the sweep scan be O(touched
-- rows) instead of a full table sweep.
CREATE INDEX IF NOT EXISTS idx_provider_tokens_codex_healthy
    ON provider_tokens (user_id)
    WHERE provider = 'codex' AND metadata->>'refresh_failure' IS NULL;
