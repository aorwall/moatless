-- One-time codes that pair a client on another device to a person's account.
--
-- A client that cannot show a browser — the T3 Code mobile app is the first —
-- has no way to reach a session cookie. It is given a short code instead: the
-- signed-in person mints one here, types it into the client, and the client
-- exchanges it once for a durable API key that acts as them from then on. The
-- code is bootstrap only. It is never the credential the client keeps.
--
-- Only the SHA-256 of the code is stored, the same way `api_keys.key_hash` is.
-- A code is displayed once, at the moment it is minted, and nothing reads it
-- back: there is no path that could show it again, so there is no reason to
-- hold the plaintext against the day the table leaks.
CREATE TABLE IF NOT EXISTS "device_pairing_codes" (
    "id" text PRIMARY KEY NOT NULL,
    "code_hash" text NOT NULL,
    -- The account the exchanged key will act as. Text, like every other
    -- `user_id` column, holding the uuid's text form.
    "user_id" text NOT NULL,
    -- Carried from the session that minted the code onto the key it becomes,
    -- so a paired device lands in the same group as the person who paired it.
    "group_id" text,
    -- What the person called the device. Becomes the API key's name, which is
    -- how they recognize it later in order to revoke it.
    "label" text,
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "expires_at" timestamptz NOT NULL,
    -- Set by the exchange itself, in the same statement that reads the row.
    -- That `UPDATE ... WHERE consumed_at IS NULL RETURNING` is what makes a
    -- code single-use across backend replicas: the second exchange of the same
    -- code updates no row and gets nothing back.
    "consumed_at" timestamptz
);

-- The exchange looks a code up by hash and by nothing else, and two codes must
-- never share one.
CREATE UNIQUE INDEX IF NOT EXISTS "device_pairing_codes_code_hash_uq"
    ON "device_pairing_codes" ("code_hash");

-- Minting prunes codes that are long finished. This is the index that read
-- walks; there is no background job, because a table that only grows while
-- people are pairing does not need one.
CREATE INDEX IF NOT EXISTS "device_pairing_codes_expires_at_idx"
    ON "device_pairing_codes" ("expires_at");
