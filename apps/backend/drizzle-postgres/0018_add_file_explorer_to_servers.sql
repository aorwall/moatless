-- Add file explorer server to all repositories that don't already have it.

-- 1. Repos with servers array but no "files" entry → prepend file explorer
UPDATE repositories
SET servers = jsonb_build_array(
  jsonb_build_object('name', 'files', 'label', 'File Explorer')
) || servers,
updated_at = NOW()
WHERE jsonb_array_length(COALESCE(servers, '[]'::jsonb)) > 0
  AND NOT EXISTS (
    SELECT 1 FROM jsonb_array_elements(servers) AS elem
    WHERE elem->>'name' = 'files'
  );

-- 2. Repos with no servers but have preview config → add files + preview
UPDATE repositories
SET servers = jsonb_build_array(
  jsonb_build_object('name', 'files', 'label', 'File Explorer'),
  jsonb_build_object(
    'name', 'preview',
    'label', 'Preview',
    'startCommands', server_start_commands,
    'startupTimeoutSeconds', startup_timeout_seconds,
    'autoReload', COALESCE(auto_reload, false),
    'enableInspector', COALESCE(enable_inspector, true)
  )
),
updated_at = NOW()
WHERE jsonb_array_length(COALESCE(servers, '[]'::jsonb)) = 0
  AND server_start_commands IS NOT NULL;

-- 3. Repos with no servers and no preview config → add just files
UPDATE repositories
SET servers = jsonb_build_array(
  jsonb_build_object('name', 'files', 'label', 'File Explorer')
),
updated_at = NOW()
WHERE jsonb_array_length(COALESCE(servers, '[]'::jsonb)) = 0
  AND server_start_commands IS NULL;
