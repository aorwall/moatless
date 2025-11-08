CREATE TABLE `provider_tokens` (
	`user_id` text NOT NULL,
	`provider` text NOT NULL,
	`auth_method` text NOT NULL,
	`access_token` text NOT NULL,
	`token_type` text,
	`scope` text,
	`provider_user_id` text,
	`provider_login` text,
	`connected_at` integer NOT NULL,
	`expires_at` integer,
	PRIMARY KEY(`user_id`, `provider`)
);
