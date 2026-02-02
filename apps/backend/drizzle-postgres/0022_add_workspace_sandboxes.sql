-- Migration: Add workspace_sandboxes table
-- Single-table replacement for workspace_runtimes.
-- Stores sandbox registry + execution target; agent/preview state queried on-demand.
CREATE TABLE IF NOT EXISTS "workspace_sandboxes" (
  "task_id" TEXT PRIMARY KEY NOT NULL,
  "mode" TEXT NOT NULL,
  "execution_target" JSONB NOT NULL DEFAULT '{}'::jsonb,
  "last_activity_at" TIMESTAMP,
  "last_error" TEXT,
  "created_at" TIMESTAMP NOT NULL DEFAULT NOW(),
  "updated_at" TIMESTAMP NOT NULL DEFAULT NOW()
);

-- Drop the old workspace_runtimes table (all state now in workspace_sandboxes)
DROP TABLE IF EXISTS "workspace_runtimes";
