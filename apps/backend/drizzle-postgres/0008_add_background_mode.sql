ALTER TABLE "tasks" ADD COLUMN IF NOT EXISTS "background" boolean;
ALTER TABLE "task_iterations" ADD COLUMN IF NOT EXISTS "background" boolean;
