-- Normalize snake_case keys in repositories.servers JSON to camelCase.
--
-- Older seeds wrote `start_commands`, `startup_timeout_seconds`, `auto_reload`,
-- and `enable_inspector`, but ServerConfig deserializes with
-- `#[serde(rename_all = "camelCase")]`, so those fields were silently dropped —
-- resulting in preview containers with no start command.

UPDATE repositories
SET servers = (
    SELECT jsonb_agg(
        (
            SELECT jsonb_object_agg(
                CASE key
                    WHEN 'start_commands' THEN 'startCommands'
                    WHEN 'startup_timeout_seconds' THEN 'startupTimeoutSeconds'
                    WHEN 'auto_reload' THEN 'autoReload'
                    WHEN 'enable_inspector' THEN 'enableInspector'
                    WHEN 'docker_image' THEN 'dockerImage'
                    WHEN 'env_vars' THEN 'envVars'
                    ELSE key
                END,
                value
            )
            FROM jsonb_each(server)
        )
    )
    FROM jsonb_array_elements(servers) AS server
)
WHERE servers IS NOT NULL
  AND jsonb_typeof(servers) = 'array'
  AND EXISTS (
      SELECT 1
      FROM jsonb_array_elements(servers) AS server
      WHERE server ? 'start_commands'
         OR server ? 'startup_timeout_seconds'
         OR server ? 'auto_reload'
         OR server ? 'enable_inspector'
         OR server ? 'docker_image'
         OR server ? 'env_vars'
  );
