-- Local username/password credentials.
--
-- `username` is the authoritative login handle for password auth and carries its
-- own uniqueness. `users.login` is deliberately left non-unique: `users.id` is
-- the provider's user id, so a GitHub `alice` and an Auth0 `alice` are two
-- legitimate rows sharing a login.
CREATE TABLE IF NOT EXISTS user_credentials (
    user_id             TEXT PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    username            TEXT NOT NULL,
    password_hash       TEXT NOT NULL,          -- argon2id PHC string
    must_change         BOOLEAN NOT NULL DEFAULT false,
    failed_attempts     INTEGER NOT NULL DEFAULT 0,
    last_login_at       TIMESTAMPTZ,
    password_changed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS user_credentials_username_uq
    ON user_credentials (lower(username));
