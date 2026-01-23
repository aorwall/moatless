-- Add background column to tasks table for background execution mode
ALTER TABLE tasks ADD COLUMN background INTEGER;--> statement-breakpoint

-- Add background column to task_iterations table
ALTER TABLE task_iterations ADD COLUMN background INTEGER;
