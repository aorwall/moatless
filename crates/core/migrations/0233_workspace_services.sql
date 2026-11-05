-- The service proxies a git-declared Workspace opts into (`services` in
-- `.moatless/workspaces.json`), as a JSON array of names. A service whose
-- availability is `granted` also needs a WORKSPACE_IDENTITY_GRANTS entry,
-- read at request time rather than stored here.
ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS services jsonb;
