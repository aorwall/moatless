-- Add visibility column for repository access control
-- Adds visibility field to control whether a repository is private (creator only) or public (all users can read)
-- Default: 'private' - only creator can read/write
-- 'public' - all authenticated users can read, only creator can write

-- Add visibility column to repositories table with default 'private'
ALTER TABLE `repositories` ADD COLUMN `visibility` TEXT DEFAULT 'private' NOT NULL CHECK(`visibility` IN ('private', 'public'));--> statement-breakpoint

-- Backfill: Set legacy repositories (null created_by) to 'public'
-- This maintains backward compatibility - legacy repos remain visible to all users
UPDATE `repositories` SET `visibility` = 'public' WHERE `created_by` IS NULL;--> statement-breakpoint

-- Create index for efficient visibility-based queries
CREATE INDEX `repositories_visibility_idx` ON `repositories`(`visibility`);
