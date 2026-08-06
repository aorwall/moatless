-- Channel pairing requests (Telegram DM access via pairing codes).

CREATE TABLE IF NOT EXISTS channel_pairing_requests (
    id serial PRIMARY KEY,
    config_id text NOT NULL REFERENCES channel_configs(id) ON DELETE CASCADE,
    provider text NOT NULL,
    provider_user_id text NOT NULL,
    code text NOT NULL,
    status text NOT NULL DEFAULT 'pending',
    meta jsonb,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now()
);

-- Only one pending request per user per config.
CREATE UNIQUE INDEX IF NOT EXISTS idx_pairing_pending_user
    ON channel_pairing_requests (config_id, provider, provider_user_id)
    WHERE status = 'pending';

-- Codes must be unique within a config while pending.
CREATE UNIQUE INDEX IF NOT EXISTS idx_pairing_pending_code
    ON channel_pairing_requests (config_id, code)
    WHERE status = 'pending';
