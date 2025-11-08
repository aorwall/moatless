CREATE TABLE `oauth_states` (
	`state` text PRIMARY KEY NOT NULL,
	`user_id` text NOT NULL,
	`provider` text NOT NULL,
	`created_at` integer NOT NULL,
	`expires_at` integer NOT NULL
);
