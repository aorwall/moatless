-- Migration: Eliminate task iterations, move per-turn state to messages
-- Add siteUrl to tasks table
ALTER TABLE "tasks" ADD COLUMN IF NOT EXISTS "site_url" TEXT;

-- Add new columns to task_messages for result messages and user message metadata
ALTER TABLE "task_messages" ADD COLUMN IF NOT EXISTS "turn_status" TEXT;
ALTER TABLE "task_messages" ADD COLUMN IF NOT EXISTS "turn_result" TEXT;
ALTER TABLE "task_messages" ADD COLUMN IF NOT EXISTS "will_not_resume" BOOLEAN;
ALTER TABLE "task_messages" ADD COLUMN IF NOT EXISTS "run_metrics" JSONB;
ALTER TABLE "task_messages" ADD COLUMN IF NOT EXISTS "agent_mode" TEXT;
ALTER TABLE "task_messages" ADD COLUMN IF NOT EXISTS "model" TEXT;
ALTER TABLE "task_messages" ADD COLUMN IF NOT EXISTS "sent_at" TIMESTAMP;

-- Rename iteration_number to turn_number
ALTER TABLE "task_messages" RENAME COLUMN "iteration_number" TO "turn_number";

-- Drop the task_iterations table
DROP TABLE IF EXISTS "task_iterations";
