-- Add target_branch column to tasks table for git checkpoint pushes
-- This stores the branch name where git checkpoints are pushed (e.g., moatless/{taskId})

ALTER TABLE tasks ADD COLUMN target_branch TEXT;
