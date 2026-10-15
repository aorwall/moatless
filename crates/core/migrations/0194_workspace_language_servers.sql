-- Language servers the sandbox starts for the agent's `LSP` tool, as a JSON
-- array of ids (`["typescript", "rust"]`). NULL starts none.
ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS language_servers jsonb;
