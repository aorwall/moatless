-- Channel extensions: cross-channel response routing
-- Adds response_config_id to channel_configs for cross-channel routing

ALTER TABLE "channel_configs"
    ADD COLUMN IF NOT EXISTS "response_config_id" text REFERENCES "channel_configs"("id");

CREATE INDEX IF NOT EXISTS "channel_configs_response_config_idx"
    ON "channel_configs" ("response_config_id");
