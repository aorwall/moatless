-- Add visibility column to tasks table with default 'private'
ALTER TABLE `tasks` ADD COLUMN `visibility` TEXT DEFAULT 'private' NOT NULL CHECK(`visibility` IN ('private', 'public'));--> statement-breakpoint

-- Backfill: Set task visibility based on repository visibility
-- Tasks inherit their repository's visibility on migration
UPDATE `tasks`
SET `visibility` = COALESCE(
  (
    SELECT `repositories`.`visibility`
    FROM `repositories`
    WHERE `repositories`.`id` = `tasks`.`repository_id`
  ),
  'private'
);--> statement-breakpoint

-- Create index for efficient visibility-based queries
CREATE INDEX `tasks_visibility_idx` ON `tasks`(`visibility`);
