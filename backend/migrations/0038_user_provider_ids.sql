CREATE TABLE user_provider_ids (
    user_id          TEXT        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    provider         TEXT        NOT NULL,   -- "slack", "telegram", …
    provider_user_id TEXT        NOT NULL,   -- e.g. "U0000000002"
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, provider)
);

-- reverse lookup: given Slack user ID → moatless user
CREATE UNIQUE INDEX user_provider_ids_lookup_idx
    ON user_provider_ids(provider, provider_user_id);
