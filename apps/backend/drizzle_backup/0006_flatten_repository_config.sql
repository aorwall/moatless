PRAGMA foreign_keys=OFF;

-- Create new repositories table with flattened structure
CREATE TABLE `repositories_new` (
  `id` text PRIMARY KEY NOT NULL,
  `name` text NOT NULL,
  `description` text,
  `path` text NOT NULL,
  `remote_url` text,
  `source_type` text,
  `use_git_worktrees` integer,
  `clone_on_new_task` integer DEFAULT false NOT NULL,
  `original_branch` text,
  `deleted` integer,
  `template_id` text,
  `vibe` integer,
  `sync_with_repo_config` integer,
  `system_prompt` text,
  -- Flattened CodeExecution fields
  `setup_commands` text,
  `env_vars` text,
  -- Flattened PreviewConfig fields
  `server_start_commands` text,
  `build_command` text,
  `build_output_directory` text DEFAULT 'dist',
  `take_screenshots` integer,
  `enable_inspector` integer DEFAULT 1,
  `startup_timeout_seconds` integer DEFAULT 30,
  -- Timestamps
  `created_at` integer NOT NULL,
  `updated_at` integer NOT NULL
);

-- Migrate data from old table to new table
INSERT INTO `repositories_new` (
  `id`,
  `name`,
  `description`,
  `path`,
  `remote_url`,
  `source_type`,
  `use_git_worktrees`,
  `clone_on_new_task`,
  `original_branch`,
  `deleted`,
  `template_id`,
  `vibe`,
  `sync_with_repo_config`,
  `system_prompt`,
  `setup_commands`,
  `env_vars`,
  `server_start_commands`,
  `build_command`,
  `build_output_directory`,
  `take_screenshots`,
  `enable_inspector`,
  `startup_timeout_seconds`,
  `created_at`,
  `updated_at`
)
SELECT
  `id`,
  `name`,
  `description`,
  `path`,
  `remote_url`,
  `source_type`,
  `use_git_worktrees`,
  `clone_on_new_task`,
  `original_branch`,
  `deleted`,
  `template_id`,
  `vibe`,
  `sync_with_repo_config`,
  `system_prompt`,
  -- Extract CodeExecution fields from JSON
  CASE
    WHEN `code_execution` IS NOT NULL AND json_valid(`code_execution`) = 1
    THEN json_extract(`code_execution`, '$.setupCommands')
    ELSE NULL
  END,
  CASE
    WHEN `code_execution` IS NOT NULL AND json_valid(`code_execution`) = 1
    THEN json_extract(`code_execution`, '$.envVars')
    ELSE NULL
  END,
  -- Extract PreviewConfig fields from JSON
  CASE
    WHEN `preview_config` IS NOT NULL AND json_valid(`preview_config`) = 1
    THEN json_extract(`preview_config`, '$.serverStartCommands')
    ELSE NULL
  END,
  CASE
    WHEN `preview_config` IS NOT NULL AND json_valid(`preview_config`) = 1
    THEN json_extract(`preview_config`, '$.buildCommand')
    ELSE NULL
  END,
  CASE
    WHEN `preview_config` IS NOT NULL AND json_valid(`preview_config`) = 1
    THEN json_extract(`preview_config`, '$.buildOutputDirectory')
    ELSE 'dist'
  END,
  CASE
    WHEN `preview_config` IS NOT NULL AND json_valid(`preview_config`) = 1
    THEN json_extract(`preview_config`, '$.takeScreenshots')
    ELSE NULL
  END,
  CASE
    WHEN `preview_config` IS NOT NULL AND json_valid(`preview_config`) = 1
    THEN json_extract(`preview_config`, '$.enableInspector')
    ELSE 1
  END,
  CASE
    WHEN `preview_config` IS NOT NULL AND json_valid(`preview_config`) = 1
    THEN json_extract(`preview_config`, '$.startupTimeoutSeconds')
    ELSE 30
  END,
  `created_at`,
  `updated_at`
FROM `repositories`;

-- Drop old table
DROP TABLE `repositories`;

-- Rename new table to original name
ALTER TABLE `repositories_new` RENAME TO `repositories`;

PRAGMA foreign_keys=ON;
