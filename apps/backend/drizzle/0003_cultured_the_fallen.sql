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
	`message` text NOT NULL,
	`tool_use_result` text,
	`attachments` text,
	`contexts` text,
	`created_at` integer DEFAULT CURRENT_TIMESTAMP NOT NULL
);
--> statement-breakpoint
CREATE INDEX `task_messages_task_idx` ON `task_messages` (`task_id`,`timestamp`,`id`);--> statement-breakpoint
CREATE UNIQUE INDEX `task_messages_uuid_idx` ON `task_messages` (`uuid`);