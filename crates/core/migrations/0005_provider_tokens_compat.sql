-- Align provider_tokens schema between Drizzle-first and Rust-first deployments.
--
-- Drizzle schema has: auth_method, scope, provider_user_id, provider_login, connected_at, expires_at
-- Rust schema has: id, metadata, created_at, updated_at
--
-- This migration adds the columns expected by provider_token_dao.rs if missing.

ALTER TABLE provider_tokens ADD COLUMN IF NOT EXISTS metadata jsonb;
ALTER TABLE provider_tokens ADD COLUMN IF NOT EXISTS created_at timestamptz DEFAULT now();
ALTER TABLE provider_tokens ADD COLUMN IF NOT EXISTS updated_at timestamptz DEFAULT now();

-- Backfill created_at from connected_at (Drizzle column) if it exists
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'provider_tokens' AND column_name = 'connected_at') THEN
        EXECUTE 'UPDATE provider_tokens SET created_at = connected_at WHERE created_at IS NULL AND connected_at IS NOT NULL';
    END IF;
END $$;

-- Backfill metadata from provider_login (Drizzle column) if it exists
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'provider_tokens' AND column_name = 'provider_login') THEN
        EXECUTE 'UPDATE provider_tokens SET metadata = jsonb_build_object(''login'', provider_login) WHERE metadata IS NULL AND provider_login IS NOT NULL';
    END IF;
END $$;
