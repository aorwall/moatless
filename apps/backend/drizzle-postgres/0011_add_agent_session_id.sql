-- Add agent_session_id column to task_messages table
-- This column tracks which agent session has acknowledged receipt of a message
-- Null means the message hasn't been received by an agent yet (pending acknowledgment)
ALTER TABLE "task_messages" ADD COLUMN IF NOT EXISTS "agent_session_id" text;--> statement-breakpoint

-- Create index for efficiently finding unacknowledged messages
CREATE INDEX IF NOT EXISTS "task_messages_unacknowledged_idx" ON "task_messages" ("task_id", "agent_session_id");
