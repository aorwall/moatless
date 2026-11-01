-- No pod refreshes its own Codex login any more: the backend is its only
-- refresher, so nothing reads which pods did.
ALTER TABLE sandboxes DROP COLUMN IF EXISTS codex_via_gateway;
