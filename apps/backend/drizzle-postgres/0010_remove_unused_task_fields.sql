-- Remove unused fields from task_iterations table
ALTER TABLE "task_iterations" DROP COLUMN IF EXISTS "git_diff";--> statement-breakpoint
ALTER TABLE "task_iterations" DROP COLUMN IF EXISTS "commit_hash";--> statement-breakpoint
ALTER TABLE "task_iterations" DROP COLUMN IF EXISTS "merged";--> statement-breakpoint
ALTER TABLE "task_iterations" DROP COLUMN IF EXISTS "git_status";--> statement-breakpoint
ALTER TABLE "task_iterations" DROP COLUMN IF EXISTS "git_changes";--> statement-breakpoint
ALTER TABLE "task_iterations" DROP COLUMN IF EXISTS "git_sync_status";--> statement-breakpoint
ALTER TABLE "task_iterations" DROP COLUMN IF EXISTS "checkpoint_commit_hash";--> statement-breakpoint
ALTER TABLE "task_iterations" DROP COLUMN IF EXISTS "checkpoint_pushed";--> statement-breakpoint

-- Remove unused fields from tasks table
ALTER TABLE "tasks" DROP COLUMN IF EXISTS "workspace_path";--> statement-breakpoint
ALTER TABLE "tasks" DROP COLUMN IF EXISTS "git_checkpoint_enabled";--> statement-breakpoint
ALTER TABLE "tasks" DROP COLUMN IF EXISTS "git_push_enabled";
