-- Add short_id column to tasks table for human-readable task identifiers
ALTER TABLE "tasks" ADD COLUMN "short_id" text;
CREATE UNIQUE INDEX "tasks_short_id_idx" ON "tasks" USING btree ("short_id");

-- Create integration_threads table for tracking external channel threads (email, slack)
CREATE TABLE "integration_threads" (
	"id" serial PRIMARY KEY NOT NULL,
	"task_id" text NOT NULL,
	"provider" text NOT NULL,
	"external_thread_id" text NOT NULL,
	"properties" jsonb NOT NULL,
	"last_message_at" timestamp NOT NULL,
	"created_at" timestamp DEFAULT now() NOT NULL
);

CREATE INDEX "integration_threads_task_idx" ON "integration_threads" USING btree ("task_id");
CREATE UNIQUE INDEX "integration_threads_provider_thread_idx" ON "integration_threads" USING btree ("provider","external_thread_id");
