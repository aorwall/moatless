-- Migration: Add sent_at column to task_messages
-- Tracks when a message was actually delivered to the agent
-- NULL means message hasn't been sent yet (queued)
ALTER TABLE "task_messages" ADD COLUMN IF NOT EXISTS "sent_at" TIMESTAMP;
