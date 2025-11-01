-- Migration: Add preview_servers table
-- Created manually due to drizzle-kit snapshot metadata conflict
-- This table stores preview server lifecycle state

CREATE TABLE `preview_servers` (
	`server_id` text PRIMARY KEY NOT NULL,
	`status` text NOT NULL,
	`port` integer,
	`url` text,
	`proxy_url` text,
	`hostname` text,
	`pid` integer,
	`inspector_enabled` integer,
	`error` text,
	`metadata` text,
	`created_at` integer NOT NULL,
	`updated_at` integer NOT NULL
);
