-- Align provider_tokens schema for mixed Rust-first and older Compose databases.
--
-- Some local databases still have the older compact provider_tokens shape and
-- are missing the auth/session fields already expected by the backend.

ALTER TABLE provider_tokens
    ADD COLUMN IF NOT EXISTS auth_method text DEFAULT 'manual';

ALTER TABLE provider_tokens
    ADD COLUMN IF NOT EXISTS scope text;

ALTER TABLE provider_tokens
    ADD COLUMN IF NOT EXISTS provider_user_id text;

ALTER TABLE provider_tokens
    ADD COLUMN IF NOT EXISTS provider_login text;

ALTER TABLE provider_tokens
    ADD COLUMN IF NOT EXISTS connected_at timestamptz DEFAULT now();

ALTER TABLE provider_tokens
    ADD COLUMN IF NOT EXISTS expires_at timestamptz;

ALTER TABLE provider_tokens
    ADD COLUMN IF NOT EXISTS team_id text;

ALTER TABLE provider_tokens
    ALTER COLUMN user_id DROP NOT NULL;

ALTER TABLE provider_tokens
    DROP CONSTRAINT IF EXISTS provider_tokens_user_provider_uq;

UPDATE provider_tokens
SET auth_method = 'manual'
WHERE auth_method IS NULL;

UPDATE provider_tokens
SET connected_at = COALESCE(created_at, now())
WHERE connected_at IS NULL;

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
        FROM pg_constraint
        WHERE conname = 'provider_tokens_scope_uq'
    ) THEN
        ALTER TABLE provider_tokens
            ADD CONSTRAINT provider_tokens_scope_uq
            UNIQUE NULLS NOT DISTINCT (user_id, provider, team_id);
    END IF;
END $$;
