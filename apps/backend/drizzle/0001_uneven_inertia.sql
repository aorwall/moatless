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
