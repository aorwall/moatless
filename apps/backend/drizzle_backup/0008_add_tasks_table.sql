-- Migration: Add tasks and task_iterations tables
-- Created manually due to drizzle-kit snapshot metadata conflict
-- These tables store task metadata and iteration history

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
CREATE UNIQUE INDEX `tasks_task_id_idx` ON `tasks` (`task_id`);--> statement-breakpoint
CREATE INDEX `tasks_status_idx` ON `tasks` (`status`);--> statement-breakpoint
CREATE INDEX `tasks_repository_idx` ON `tasks` (`repository_id`);--> statement-breakpoint
CREATE INDEX `tasks_last_message_idx` ON `tasks` (`last_message_at`);--> statement-breakpoint
CREATE INDEX `task_iterations_task_iteration_idx` ON `task_iterations` (`task_id`,`iteration_number`);--> statement-breakpoint
CREATE INDEX `task_iterations_status_idx` ON `task_iterations` (`status`);
