-- Remove background columns (no longer needed after removing background mode)
ALTER TABLE "tasks" DROP COLUMN IF EXISTS "background";
ALTER TABLE "task_iterations" DROP COLUMN IF EXISTS "background";
