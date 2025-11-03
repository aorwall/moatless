CREATE TABLE `sessions` (
	`id` text PRIMARY KEY NOT NULL,
	`provider` text NOT NULL,
	`user_id` text NOT NULL,
	`user_login` text NOT NULL,
	`user_email` text,
	`user_name` text,
	`avatar_url` text,
	`access_token` text,
	`id_token` text,
	`token_type` text,
	`scope` text,
	`group_id` text,
	`created_at` integer NOT NULL,
	`expires_at` integer NOT NULL
);
--> statement-breakpoint
CREATE TABLE `users` (
	`id` text PRIMARY KEY NOT NULL,
	`login` text NOT NULL,
	`email` text,
	`name` text,
	`avatar_url` text,
	`provider` text NOT NULL,
	`created_at` integer NOT NULL,
	`last_login_at` integer NOT NULL
);
--> statement-breakpoint
CREATE TABLE `repositories` (
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
	`setup_commands` text,
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
CREATE TABLE `task_message_contexts` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`task_message_id` integer NOT NULL,
	`type` text NOT NULL,
	`reference_id` text NOT NULL,
	`data` text NOT NULL,
	`created_at` integer DEFAULT CURRENT_TIMESTAMP NOT NULL,
	FOREIGN KEY (`task_message_id`) REFERENCES `task_messages`(`id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE INDEX `task_message_contexts_message_idx` ON `task_message_contexts` (`task_message_id`);--> statement-breakpoint
CREATE INDEX `task_message_contexts_reference_idx` ON `task_message_contexts` (`reference_id`);--> statement-breakpoint
CREATE TABLE `task_messages` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`task_id` text NOT NULL,
	`uuid` text NOT NULL,
	`message_type` text NOT NULL,
	`user_type` text NOT NULL,
	`parent_uuid` text,
	`is_sidechain` integer,
	`cwd` text,
	`session_id` text,
	`iteration_number` integer,
	`version` text,
	`git_branch` text,
	`request_id` text,
	`timestamp` text NOT NULL,
	`message_role` text NOT NULL,
	`message_content` text NOT NULL,
	`assistant_message_id` text,
	`assistant_message_type` text,
	`assistant_message_model` text,
	`assistant_stop_reason` text,
	`assistant_stop_sequence` text,
	`assistant_usage_input_tokens` integer,
	`assistant_usage_output_tokens` integer,
	`assistant_usage_cache_creation_input_tokens` integer,
	`assistant_usage_cache_read_input_tokens` integer,
	`tool_use_result` text,
	`attachments` text,
	`created_at` integer DEFAULT CURRENT_TIMESTAMP NOT NULL
);
--> statement-breakpoint
CREATE INDEX `task_messages_task_idx` ON `task_messages` (`task_id`,`timestamp`,`id`);--> statement-breakpoint
CREATE UNIQUE INDEX `task_messages_uuid_idx` ON `task_messages` (`uuid`);--> statement-breakpoint
CREATE TABLE `task_iterations` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`task_id` text NOT NULL,
	`iteration_number` integer NOT NULL,
	`start_time` text NOT NULL,
	`end_time` text,
	`status` text NOT NULL,
	`message_count` integer NOT NULL,
	`cost_usd` real NOT NULL,
	`context_tokens` integer,
	`initial_message` text,
	`command_name` text,
	`result` text,
	`git_diff` text,
	`commit_hash` text,
	`merged` integer,
	`agent_session_id` text,
	`model` text,
	`agent_mode` text,
	`git_status` text,
	`git_changes` text,
	`git_sync_status` text,
	`will_not_resume` integer,
	`screenshot_path` text,
	`thumbnail_path` text,
	`site_url` text,
	`checkpoint_commit_hash` text,
	`checkpoint_pushed` integer,
	`created_at` integer DEFAULT CURRENT_TIMESTAMP NOT NULL,
	FOREIGN KEY (`task_id`) REFERENCES `tasks`(`task_id`) ON UPDATE no action ON DELETE cascade
);
--> statement-breakpoint
CREATE INDEX `task_iterations_task_iteration_idx` ON `task_iterations` (`task_id`,`iteration_number`);--> statement-breakpoint
CREATE INDEX `task_iterations_status_idx` ON `task_iterations` (`status`);--> statement-breakpoint
CREATE TABLE `tasks` (
	`id` integer PRIMARY KEY AUTOINCREMENT NOT NULL,
	`task_id` text NOT NULL,
	`created_at` text NOT NULL,
	`last_message_at` text NOT NULL,
	`agent_type` text NOT NULL,
	`repository_id` text NOT NULL,
	`status` text NOT NULL,
	`num_turns` integer NOT NULL,
	`total_cost_usd` real NOT NULL,
	`workspace_path` text,
	`template_id` text,
	`vibe` integer,
	`name` text,
	`description` text,
	`site_id` text,
	`site_provider` text,
	`git_checkpoint_enabled` integer,
	`git_push_enabled` integer,
	`created_at_db` integer DEFAULT CURRENT_TIMESTAMP NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX `tasks_task_id_idx` ON `tasks` (`task_id`);--> statement-breakpoint
CREATE INDEX `tasks_status_idx` ON `tasks` (`status`);--> statement-breakpoint
CREATE INDEX `tasks_repository_idx` ON `tasks` (`repository_id`);--> statement-breakpoint
CREATE INDEX `tasks_last_message_idx` ON `tasks` (`last_message_at`);--> statement-breakpoint
CREATE TABLE `workspace_runtimes` (
	`task_id` text PRIMARY KEY NOT NULL,
	`repository_id` text NOT NULL,
	`config_revision` text NOT NULL,
	`execution_target` text NOT NULL,
	`agent` text,
	`preview` text NOT NULL,
	`locks` text,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL
);
