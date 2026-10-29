-- Whether the Sandbox's Codex reaches its model through the backend's model
-- gateway, as its pod last reported on `GetCredentials`. FALSE for a pod that
-- holds its own `auth.json` and refreshes it, which the gateway's refresh
-- yields to.
ALTER TABLE sandboxes ADD COLUMN IF NOT EXISTS codex_via_gateway boolean NOT NULL DEFAULT false;
