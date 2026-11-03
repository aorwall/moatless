-- A key with `idle_ttl_secs` set expires only after going that long unused:
-- each use moves `expires_at` to that far past it (`api_key::validate_key`).
-- Paired devices get one, so a phone in regular use stays paired; a key a
-- person mints keeps the fixed expiry they chose, so the column is nullable.
ALTER TABLE api_keys ADD COLUMN IF NOT EXISTS idle_ttl_secs bigint;

-- Keys already paired. Nothing links a key to the code it was redeemed from,
-- but only pairing mints a scoped key exactly 90 days from its creation. An
-- expired key stays expired: validation refuses it before it can be renewed.
UPDATE api_keys
SET idle_ttl_secs = 7776000
WHERE idle_ttl_secs IS NULL
  AND scopes IS NOT NULL
  AND expires_at > now()
  AND expires_at - created_at BETWEEN interval '89 days 23 hours' AND interval '90 days 1 hour';
