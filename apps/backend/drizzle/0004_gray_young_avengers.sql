PRAGMA foreign_keys=OFF;--> statement-breakpoint
CREATE TABLE `__new_repositories` (
	`id` text PRIMARY KEY NOT NULL,
	`kind` text NOT NULL,
	`name` text NOT NULL,
	`description` text,
	`path` text,
	`remote_url` text,
	`provider` text,
	`default_branch` text,
	`template_id` text,
	`use_git_worktrees` integer,
	`original_branch` text,
	`source_type` text,
	`clone_on_new_task` integer,
	`deleted` integer,
	`vibe` integer,
	`sync_with_repo_config` integer,
	`system_prompt` text,
	`setup_commands` text,
	`docker_image` text,
	`env_vars` text,
	`server_start_commands` text,
	`build_command` text,
	`build_output_directory` text DEFAULT 'dist',
	`take_screenshots` integer,
	`enable_inspector` integer DEFAULT true,
	`startup_timeout_seconds` integer DEFAULT 30,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL
);
--> statement-breakpoint
-- Migrate existing data with proper kind classification
-- Template repositories: have template_id set
INSERT INTO `__new_repositories` SELECT
  "id",
  CASE
    WHEN "template_id" IS NOT NULL THEN 'template'
    WHEN "clone_on_new_task" = 1 AND "remote_url" IS NOT NULL THEN 'remote'
    ELSE 'local'
  END as "kind",
  "name", "description",
  -- Set path to NULL for remote repositories
  CASE
    WHEN "clone_on_new_task" = 1 AND "remote_url" IS NOT NULL AND "template_id" IS NULL THEN NULL
    ELSE "path"
  END as "path",
  "remote_url",
  -- Set provider to 'github' for remote repositories
  CASE
    WHEN "clone_on_new_task" = 1 AND "remote_url" IS NOT NULL AND "template_id" IS NULL THEN 'github'
    ELSE NULL
  END as "provider",
  NULL as "default_branch",
  "template_id",
  -- Set use_git_worktrees to NULL for remote repositories
  CASE
    WHEN "clone_on_new_task" = 1 AND "remote_url" IS NOT NULL AND "template_id" IS NULL THEN NULL
    ELSE "use_git_worktrees"
  END as "use_git_worktrees",
  "original_branch", "source_type", "clone_on_new_task", "deleted", "vibe",
  "sync_with_repo_config", "system_prompt", "setup_commands",
  NULL as "docker_image",
  "env_vars", "server_start_commands", "build_command", "build_output_directory",
  "take_screenshots", "enable_inspector", "startup_timeout_seconds",
  "created_at", "updated_at"
FROM `repositories`;
--> statement-breakpoint
DROP TABLE `repositories`;--> statement-breakpoint
ALTER TABLE `__new_repositories` RENAME TO `repositories`;--> statement-breakpoint
PRAGMA foreign_keys=ON;
