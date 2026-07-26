-- Per-task live overrides for preview servers: start command and env, keyed by
-- server name. Stored per-task (not on workspaces.servers) so reconfiguring a
-- server for one sandbox does not affect other tasks sharing the workspace,
-- and so the override survives stop/start and redeploy — it is replayed into
-- the sandbox's control files at init.
--
-- Shape: {"<server>": {"startCommand": "...", "env": {...}, "unsetEnv": [...]}}
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS server_overrides JSONB;
