-- Mark bot users (rows in `users` that represent a GitHub App's `{slug}[bot]`
-- identity) so they can be referenced by `trigger_rules.run_as_user_id`. The
-- App key + bot identity itself live in `provider_tokens`:
--   provider = 'github'
--   auth_method = 'github_app_installation'
--   provider_user_id = <app key, e.g. "dev-bot">
--   provider_login = '<slug>[bot]'
-- Credential resolution branches on the auth_method to mint an installation
-- token instead of refreshing an OAuth token.
ALTER TABLE users
    ADD COLUMN IF NOT EXISTS is_bot BOOLEAN NOT NULL DEFAULT false;

CREATE INDEX IF NOT EXISTS idx_users_is_bot ON users(is_bot) WHERE is_bot;
