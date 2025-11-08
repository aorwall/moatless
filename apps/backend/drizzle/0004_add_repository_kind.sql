-- Migration: Add repository kind discriminator and provider fields
-- This migration introduces the discriminated union pattern for repositories

-- Step 1: Add kind column with default 'local'
ALTER TABLE `repositories` ADD COLUMN `kind` text NOT NULL DEFAULT 'local';
--> statement-breakpoint

-- Step 2: Add provider and default_branch columns for remote repositories
ALTER TABLE `repositories` ADD COLUMN `provider` text;
--> statement-breakpoint
ALTER TABLE `repositories` ADD COLUMN `default_branch` text;
--> statement-breakpoint

-- Step 3: Add docker_image column (part of code execution flattening)
ALTER TABLE `repositories` ADD COLUMN `docker_image` text;
--> statement-breakpoint

-- Step 4: Backfill template repositories
-- Set kind = 'template' where template_id is not null
UPDATE `repositories`
SET `kind` = 'template'
WHERE `template_id` IS NOT NULL;
--> statement-breakpoint

-- Step 5: Backfill remote repositories
-- Set kind = 'remote' for repos that were configured to clone on new task
-- These had both clone_on_new_task=1 and remote_url set
UPDATE `repositories`
SET `kind` = 'remote',
    `provider` = 'github',
    `path` = NULL,
    `use_git_worktrees` = NULL
WHERE `clone_on_new_task` = 1
  AND `remote_url` IS NOT NULL
  AND `template_id` IS NULL;
--> statement-breakpoint

-- Step 6: All remaining repositories are local (already have kind='local' from default)
-- No additional update needed

-- Note: We're keeping clone_on_new_task and source_type columns for now
-- to maintain backward compatibility. They will be marked as deprecated
-- and removed in a future migration after all code is updated.
