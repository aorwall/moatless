-- Migrate previewConfig to servers array for existing repositories
-- Only migrates repos that have preview config data but no servers configured yet
UPDATE repositories
SET servers = jsonb_build_array(
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
WHERE server_start_commands IS NOT NULL
  AND jsonb_array_length(COALESCE(servers, '[]'::jsonb)) = 0;
