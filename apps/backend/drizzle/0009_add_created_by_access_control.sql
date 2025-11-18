-- Add createdBy columns for user-scoped access control
-- Adds optional createdBy field to repositories and tasks tables
-- Nullable to support legacy data (records without createdBy are visible to all users)

-- Add createdBy to repositories table
ALTER TABLE `repositories` ADD COLUMN `created_by` TEXT;--> statement-breakpoint
-- Add createdBy to tasks table
ALTER TABLE `tasks` ADD COLUMN `created_by` TEXT;--> statement-breakpoint
-- Create indexes for efficient user-scoped queries
CREATE INDEX `repositories_created_by_idx` ON `repositories`(`created_by`);--> statement-breakpoint
CREATE INDEX `tasks_created_by_idx` ON `tasks`(`created_by`);
