CREATE TABLE `commands` (
	`name` text PRIMARY KEY NOT NULL,
	`description` text NOT NULL,
	`prompt` text NOT NULL,
	`conditions` text,
	`icon` text,
	`variant` text,
	`active` integer,
	`will_not_resume` integer,
	`agent_mode` text
);
--> statement-breakpoint
CREATE INDEX `commands_active_idx` ON `commands` (`active`);