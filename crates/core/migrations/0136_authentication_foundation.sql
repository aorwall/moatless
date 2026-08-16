BEGIN;

-- Interactive human accounts are addressable by a normalized email. Bots keep
-- their provider-owned handles and are not permitted to authenticate through
-- browser login mechanisms.
ALTER TABLE users
    ADD COLUMN IF NOT EXISTS email_normalized text
        GENERATED ALWAYS AS (lower(btrim(email))) STORED,
    ADD COLUMN IF NOT EXISTS status text NOT NULL DEFAULT 'active';

DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM users WHERE NOT is_bot AND email IS NULL) THEN
        RAISE EXCEPTION 'every human user must have an email before the authentication migration';
    END IF;
    IF EXISTS (
        SELECT lower(btrim(email))
        FROM users
        WHERE NOT is_bot
        GROUP BY lower(btrim(email))
        HAVING count(*) > 1
    ) THEN
        RAISE EXCEPTION 'human user emails must be unique before the authentication migration';
    END IF;
END $$;

ALTER TABLE users
    ADD CONSTRAINT users_human_email_required
        CHECK (is_bot OR email IS NOT NULL),
    ADD CONSTRAINT users_status_check
        CHECK (status IN ('active', 'disabled'));

CREATE UNIQUE INDEX users_human_email_normalized_uq
    ON users (email_normalized)
    WHERE NOT is_bot;

-- Login identities are separate from Adapter sender mappings in
-- user_provider_ids. OIDC identity is always the canonical issuer + sub pair.
CREATE TABLE oidc_identities (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider_id text NOT NULL,
    issuer text NOT NULL,
    subject text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    last_login_at timestamptz,
    UNIQUE (issuer, subject),
    UNIQUE (user_id, issuer)
);

-- JWT cookies and the legacy token snapshot table cannot provide revocation.
-- The maintenance cutover intentionally invalidates every browser login.
DROP TABLE sessions;

CREATE TABLE auth_sessions (
    id uuid PRIMARY KEY,
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash bytea NOT NULL UNIQUE,
    csrf_token_hash bytea NOT NULL,
    authenticated_via text NOT NULL,
    group_id text,
    created_at timestamptz NOT NULL,
    last_seen_at timestamptz NOT NULL,
    idle_expires_at timestamptz NOT NULL,
    absolute_expires_at timestamptz NOT NULL,
    revoked_at timestamptz,
    ip_prefix inet,
    user_agent text,
    CONSTRAINT auth_sessions_lifetime_check
        CHECK (idle_expires_at <= absolute_expires_at)
);

CREATE INDEX auth_sessions_user_active_idx
    ON auth_sessions (user_id)
    WHERE revoked_at IS NULL;
CREATE INDEX auth_sessions_expiry_idx
    ON auth_sessions (absolute_expires_at)
    WHERE revoked_at IS NULL;

CREATE TABLE auth_transactions (
    id uuid PRIMARY KEY,
    provider_id text NOT NULL,
    state_hash bytea NOT NULL UNIQUE,
    nonce text NOT NULL,
    pkce_verifier text NOT NULL,
    redirect_after_login text,
    cli_callback_port integer,
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz
);

CREATE INDEX auth_transactions_expiry_idx ON auth_transactions (expires_at);

CREATE TABLE auth_rate_limits (
    bucket text NOT NULL,
    subject_hash bytea NOT NULL,
    window_started_at timestamptz NOT NULL,
    attempts integer NOT NULL DEFAULT 0,
    PRIMARY KEY (bucket, subject_hash, window_started_at)
);

CREATE INDEX auth_rate_limits_window_idx ON auth_rate_limits (window_started_at);

CREATE TABLE auth_audit_events (
    id uuid PRIMARY KEY,
    occurred_at timestamptz NOT NULL DEFAULT now(),
    user_id uuid REFERENCES users(id) ON DELETE SET NULL,
    auth_session_id uuid REFERENCES auth_sessions(id) ON DELETE SET NULL,
    event text NOT NULL,
    authentication_method text,
    provider_id text,
    success boolean NOT NULL,
    ip_prefix inet,
    metadata jsonb NOT NULL DEFAULT '{}'::jsonb
);

CREATE INDEX auth_audit_events_user_time_idx
    ON auth_audit_events (user_id, occurred_at DESC);

COMMIT;
