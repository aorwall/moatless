-- api_keys reads and writes `scopes` and `group_id` (backend/src/auth/api_keys.rs),
-- but 0001_schema.sql created the table without them. On a database migrated from
-- scratch every `SELECT * FROM api_keys` fails to map into `ApiKey`, so no API key
-- can be minted or validated. Add both columns.
--
-- `scopes` holds the JSON-encoded `Vec<String>` written at api_key.rs (serde_json::to_string)
-- and decoded back on listing; a plain nullable text column matches what is bound.
-- `group_id` is a plain nullable text tag with no FK (same as users.group_id and
-- device_pairing_codes.group_id).
ALTER TABLE api_keys ADD COLUMN IF NOT EXISTS scopes text;
ALTER TABLE api_keys ADD COLUMN IF NOT EXISTS group_id text;
