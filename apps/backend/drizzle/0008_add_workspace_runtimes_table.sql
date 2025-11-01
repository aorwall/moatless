-- Migration: Add workspace_runtimes table
-- Created manually due to drizzle-kit snapshot metadata conflict
-- This table stores workspace runtime state

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
