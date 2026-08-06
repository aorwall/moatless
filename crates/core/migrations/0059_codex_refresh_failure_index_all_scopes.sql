-- Broaden the codex refresher sweep index to cover team / global rows.
--
-- Migration 0058 created the partial index `idx_provider_tokens_codex_healthy`
-- on `(user_id) WHERE provider = 'codex' AND metadata->>'refresh_failure' IS NULL`.
-- That worked when codex refresh was user-scope only. With team + global codex
-- rows landing in this release, the sweep now reads every healthy codex row
-- regardless of scope; the user_id-only index column is no longer the right
-- shape (team / global rows have `user_id IS NULL` and would be excluded from
-- the b-tree key entirely, defeating the index for those scopes).
--
-- Recreate the index on `(team_id, scope, user_id)` — every codex row has
-- a non-null `team_id` (personal-{user} for user rows, the team id for team
-- rows, 'system' for global rows) so the b-tree key is dense, and the trailing
-- `(scope, user_id)` columns keep both scope-filtered scans and the user-row
-- lookup path cheap.

DROP INDEX IF EXISTS idx_provider_tokens_codex_healthy;

CREATE INDEX IF NOT EXISTS idx_provider_tokens_codex_healthy_all_scopes
    ON provider_tokens (team_id, scope, user_id)
    WHERE provider = 'codex' AND metadata->>'refresh_failure' IS NULL;
