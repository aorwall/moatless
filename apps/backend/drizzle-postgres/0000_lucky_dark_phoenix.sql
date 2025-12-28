CREATE TABLE "commands" (
	"name" text PRIMARY KEY NOT NULL,
	"description" text NOT NULL,
	"prompt" text NOT NULL,
	"conditions" jsonb,
	"icon" text,
	"variant" text,
	"active" boolean,
	"will_not_resume" boolean,
	"agent_mode" text
);
--> statement-breakpoint
CREATE TABLE "oauth_states" (
	"state" text PRIMARY KEY NOT NULL,
	"user_id" text NOT NULL,
	"provider" text NOT NULL,
	"created_at" timestamp NOT NULL,
	"expires_at" timestamp NOT NULL
);
--> statement-breakpoint
CREATE TABLE "provider_tokens" (
	"user_id" text NOT NULL,
	"provider" text NOT NULL,
	"auth_method" text NOT NULL,
	"access_token" text NOT NULL,
	"refresh_token" text,
	"token_type" text,
	"scope" text,
	"provider_user_id" text,
	"provider_login" text,
	"connected_at" timestamp NOT NULL,
	"expires_at" timestamp,
	CONSTRAINT "provider_tokens_user_id_provider_pk" PRIMARY KEY("user_id","provider")
);
--> statement-breakpoint
CREATE TABLE "sessions" (
	"id" text PRIMARY KEY NOT NULL,
	"provider" text NOT NULL,
	"user_id" text NOT NULL,
	"user_login" text NOT NULL,
	"user_email" text,
	"user_name" text,
	"avatar_url" text,
	"access_token" text,
	"id_token" text,
	"token_type" text,
	"scope" text,
	"group_id" text,
	"created_at" timestamp NOT NULL,
	"expires_at" timestamp NOT NULL
);
--> statement-breakpoint
CREATE TABLE "users" (
	"id" text PRIMARY KEY NOT NULL,
	"login" text NOT NULL,
	"email" text,
	"name" text,
	"avatar_url" text,
	"provider" text NOT NULL,
	"created_at" timestamp NOT NULL,
	"last_login_at" timestamp NOT NULL
);
--> statement-breakpoint
CREATE TABLE "repositories" (
	"id" text PRIMARY KEY NOT NULL,
	"kind" text NOT NULL,
	"name" text NOT NULL,
	"description" text,
	"path" text,
	"template_id" text,
	"remote_url" text,
	"provider" text,
	"default_branch" text,
	"use_git_worktrees" boolean,
	"original_branch" text,
	"source_type" text,
	"clone_on_new_task" boolean,
	"deleted" boolean,
	"vibe" boolean,
	"system_prompt" text,
	"setup_commands" jsonb,
	"docker_image" text,
	"env_vars" jsonb,
	"server_start_commands" jsonb,
	"build_command" text,
	"build_output_directory" text DEFAULT 'dist',
	"take_screenshots" boolean,
	"enable_inspector" boolean DEFAULT true,
	"startup_timeout_seconds" integer DEFAULT 30,
	"auto_reload" boolean,
	"created_by" text,
	"visibility" text DEFAULT 'private' NOT NULL,
	"created_at" timestamp NOT NULL,
	"updated_at" timestamp NOT NULL
);
--> statement-breakpoint
CREATE TABLE "task_message_contexts" (
	"id" serial PRIMARY KEY NOT NULL,
	"task_message_id" integer NOT NULL,
	"type" text NOT NULL,
	"reference_id" text NOT NULL,
	"data" jsonb NOT NULL,
	"created_at" timestamp DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "task_messages" (
	"id" serial PRIMARY KEY NOT NULL,
	"task_id" text NOT NULL,
	"uuid" text NOT NULL,
	"message_type" text NOT NULL,
	"user_type" text NOT NULL,
	"parent_uuid" text,
	"is_sidechain" boolean,
	"cwd" text,
	"session_id" text,
	"iteration_number" integer,
	"version" text,
	"git_branch" text,
	"request_id" text,
	"timestamp" text NOT NULL,
	"message_role" text NOT NULL,
	"message_content" jsonb NOT NULL,
	"assistant_message_id" text,
	"assistant_message_type" text,
	"assistant_message_model" text,
	"assistant_stop_reason" text,
	"assistant_stop_sequence" text,
	"assistant_usage_input_tokens" integer,
	"assistant_usage_output_tokens" integer,
	"assistant_usage_cache_creation_input_tokens" integer,
	"assistant_usage_cache_read_input_tokens" integer,
	"tool_use_result" jsonb,
	"attachments" jsonb,
	"created_at" timestamp DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "task_iterations" (
	"id" serial PRIMARY KEY NOT NULL,
	"task_id" text NOT NULL,
	"iteration_number" integer NOT NULL,
	"start_time" text NOT NULL,
	"end_time" text,
	"status" text NOT NULL,
	"message_count" integer NOT NULL,
	"cost_usd" double precision NOT NULL,
	"context_tokens" integer,
	"initial_message" text,
	"command_name" text,
	"result" text,
	"git_diff" text,
	"commit_hash" text,
	"merged" boolean,
	"agent_session_id" text,
	"model" text,
	"agent_mode" text,
	"git_status" jsonb,
	"git_changes" jsonb,
	"git_sync_status" jsonb,
	"will_not_resume" boolean,
	"screenshot_path" text,
	"thumbnail_path" text,
	"screenshot_file_id" text,
	"site_url" text,
	"checkpoint_commit_hash" text,
	"checkpoint_pushed" boolean,
	"created_at" timestamp DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "tasks" (
	"id" serial PRIMARY KEY NOT NULL,
	"task_id" text NOT NULL,
	"created_at" text NOT NULL,
	"last_message_at" text NOT NULL,
	"agent_type" text NOT NULL,
	"repository_id" text NOT NULL,
	"status" text NOT NULL,
	"num_turns" integer NOT NULL,
	"total_cost_usd" double precision NOT NULL,
	"workspace_path" text,
	"template_id" text,
	"vibe" boolean,
	"name" text,
	"description" text,
	"site_id" text,
	"site_provider" text,
	"git_checkpoint_enabled" boolean,
	"git_push_enabled" boolean,
	"created_by" text,
	"visibility" text DEFAULT 'private' NOT NULL,
	"created_at_db" timestamp DEFAULT now() NOT NULL
);
--> statement-breakpoint
CREATE TABLE "workspace_runtimes" (
	"task_id" text PRIMARY KEY NOT NULL,
	"repository_id" text NOT NULL,
	"config_revision" text NOT NULL,
	"execution_target" text NOT NULL,
	"agent" text,
	"preview" text NOT NULL,
	"locks" text,
	"created_at" timestamp NOT NULL,
	"updated_at" timestamp NOT NULL
);
--> statement-breakpoint
CREATE UNIQUE INDEX "tasks_task_id_idx" ON "tasks" USING btree ("task_id");--> statement-breakpoint
CREATE UNIQUE INDEX "task_messages_uuid_idx" ON "task_messages" USING btree ("uuid");--> statement-breakpoint
ALTER TABLE "task_message_contexts" ADD CONSTRAINT "task_message_contexts_task_message_id_task_messages_id_fk" FOREIGN KEY ("task_message_id") REFERENCES "public"."task_messages"("id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
ALTER TABLE "task_iterations" ADD CONSTRAINT "task_iterations_task_id_tasks_task_id_fk" FOREIGN KEY ("task_id") REFERENCES "public"."tasks"("task_id") ON DELETE cascade ON UPDATE no action;--> statement-breakpoint
CREATE INDEX "commands_active_idx" ON "commands" USING btree ("active");--> statement-breakpoint
CREATE INDEX "repositories_created_by_idx" ON "repositories" USING btree ("created_by");--> statement-breakpoint
CREATE INDEX "repositories_visibility_idx" ON "repositories" USING btree ("visibility");--> statement-breakpoint
CREATE INDEX "task_message_contexts_message_idx" ON "task_message_contexts" USING btree ("task_message_id");--> statement-breakpoint
CREATE INDEX "task_message_contexts_reference_idx" ON "task_message_contexts" USING btree ("reference_id");--> statement-breakpoint
CREATE INDEX "task_messages_task_idx" ON "task_messages" USING btree ("task_id","timestamp","id");--> statement-breakpoint
CREATE INDEX "task_iterations_task_iteration_idx" ON "task_iterations" USING btree ("task_id","iteration_number");--> statement-breakpoint
CREATE INDEX "task_iterations_status_idx" ON "task_iterations" USING btree ("status");--> statement-breakpoint
CREATE INDEX "tasks_status_idx" ON "tasks" USING btree ("status");--> statement-breakpoint
CREATE INDEX "tasks_repository_idx" ON "tasks" USING btree ("repository_id");--> statement-breakpoint
CREATE INDEX "tasks_last_message_idx" ON "tasks" USING btree ("last_message_at");--> statement-breakpoint
CREATE INDEX "tasks_created_by_idx" ON "tasks" USING btree ("created_by");--> statement-breakpoint
CREATE INDEX "tasks_visibility_idx" ON "tasks" USING btree ("visibility");