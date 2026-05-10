-- Consolidated schema for sqlx::test
-- Derived from Drizzle migrations 0000–0027 (final state after all ALTERs)
-- Idempotent: safe to run against an existing Drizzle-managed database.

-- =========================================================================
-- Users
-- =========================================================================
CREATE TABLE IF NOT EXISTS "users" (
    "id" text PRIMARY KEY NOT NULL,
    "login" text NOT NULL,
    "email" text,
    "name" text,
    "avatar_url" text,
    "provider" text NOT NULL,
    "role" text NOT NULL DEFAULT 'user',
    "created_at" timestamptz NOT NULL,
    "last_login_at" timestamptz NOT NULL
);

-- =========================================================================
-- Sessions
-- =========================================================================
CREATE TABLE IF NOT EXISTS "sessions" (
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
    "role" text NOT NULL DEFAULT 'user',
    "created_at" timestamptz NOT NULL,
    "expires_at" timestamptz NOT NULL
);

-- =========================================================================
-- API keys
-- =========================================================================
CREATE TABLE IF NOT EXISTS "api_keys" (
    "id" text PRIMARY KEY NOT NULL,
    "user_id" text NOT NULL,
    "name" text NOT NULL,
    "key_hash" text NOT NULL,
    "key_prefix" text NOT NULL,
    "created_at" timestamptz NOT NULL,
    "last_used_at" timestamptz,
    "expires_at" timestamptz,
    "revoked" boolean DEFAULT false
);

-- =========================================================================
-- Provider tokens
-- =========================================================================
CREATE TABLE IF NOT EXISTS "provider_tokens" (
    "id" serial PRIMARY KEY NOT NULL,
    "user_id" text NOT NULL,
    "provider" text NOT NULL,
    "access_token" text NOT NULL,
    "refresh_token" text,
    "token_type" text,
    "metadata" jsonb,
    "created_at" timestamptz DEFAULT now(),
    "updated_at" timestamptz DEFAULT now(),
    CONSTRAINT "provider_tokens_user_provider_uq" UNIQUE ("user_id", "provider")
);

-- =========================================================================
-- Repositories
-- =========================================================================
CREATE TABLE IF NOT EXISTS "repositories" (
    "id" text PRIMARY KEY NOT NULL,
    "kind" text NOT NULL,
    "name" text NOT NULL,
    "description" text,
    "remote_url" text,
    "provider" text,
    "default_branch" text,
    "deleted" boolean,
    "system_prompt" text,
    "setup_commands" jsonb,
    "docker_image" text,
    "env_vars" jsonb,
    "server_start_commands" jsonb,
    "build_command" text,
    "build_output_directory" text DEFAULT 'dist',
    "enable_inspector" boolean DEFAULT true,
    "startup_timeout_seconds" integer DEFAULT 30,
    "auto_reload" boolean,
    "created_by" text,
    "scope" text DEFAULT 'user' NOT NULL,
    "team_id" text DEFAULT 'system' NOT NULL,
    "owner_user_id" text,
    "created_at" timestamptz,
    "updated_at" timestamptz,
    "servers" jsonb DEFAULT '[]'::jsonb,
    "plugins" jsonb
);

CREATE INDEX IF NOT EXISTS "repositories_created_by_idx" ON "repositories" USING btree ("created_by");
CREATE INDEX IF NOT EXISTS "repositories_scope_team_owner_idx" ON "repositories" ("scope", "team_id", "owner_user_id");

-- =========================================================================
-- Tasks
-- =========================================================================
CREATE TABLE IF NOT EXISTS "tasks" (
    "id" serial,
    "task_id" text NOT NULL,
    "short_id" text,
    "created_at" text NOT NULL,
    "last_message_at" text NOT NULL,
    "agent_type" text NOT NULL,
    "repository_id" text NOT NULL,
    "status" text NOT NULL,
    "total_cost_usd" double precision NOT NULL,
    "site_url" text,
    "template_id" text,
    "vibe" boolean,
    "name" text,
    "description" text,
    "site_id" text,
    "site_provider" text,
    "branch" text,
    "created_by" text,
    "visibility" text DEFAULT 'private' NOT NULL,
    "scope" text DEFAULT 'user' NOT NULL,
    "team_id" text DEFAULT 'system' NOT NULL,
    "owner_user_id" text,
    "created_at_db" timestamptz DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS "tasks_task_id_idx" ON "tasks" USING btree ("task_id");
CREATE UNIQUE INDEX IF NOT EXISTS "tasks_short_id_idx" ON "tasks" USING btree ("short_id");
CREATE INDEX IF NOT EXISTS "tasks_status_idx" ON "tasks" USING btree ("status");
CREATE INDEX IF NOT EXISTS "tasks_repository_idx" ON "tasks" USING btree ("repository_id");
CREATE INDEX IF NOT EXISTS "tasks_last_message_idx" ON "tasks" USING btree ("last_message_at");
CREATE INDEX IF NOT EXISTS "tasks_created_by_idx" ON "tasks" USING btree ("created_by");
CREATE INDEX IF NOT EXISTS "tasks_visibility_idx" ON "tasks" USING btree ("visibility");

-- =========================================================================
-- Task messages
-- =========================================================================
CREATE TABLE IF NOT EXISTS "task_messages" (
    "id" serial PRIMARY KEY NOT NULL,
    "task_id" text NOT NULL,
    "uuid" text NOT NULL,
    "message_type" text NOT NULL,
    "user_type" text NOT NULL,
    "parent_uuid" text,
    "is_sidechain" boolean,
    "cwd" text,
    "session_id" text,
    "turn_number" integer,
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
    "agent_session_id" text,
    "turn_status" text,
    "turn_result" text,
    "will_not_resume" boolean,
    "run_metrics" jsonb,
    "agent_mode" text,
    "model" text,
    "resume_session_id" text,
    "sent_at" timestamptz,
    "created_at" timestamptz DEFAULT now() NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS "task_messages_uuid_idx" ON "task_messages" USING btree ("uuid");
CREATE INDEX IF NOT EXISTS "task_messages_task_idx" ON "task_messages" USING btree ("task_id", "timestamp", "id");
CREATE INDEX IF NOT EXISTS "task_messages_unacknowledged_idx" ON "task_messages" ("task_id", "agent_session_id");

-- =========================================================================
-- Task message contexts
-- =========================================================================
CREATE TABLE IF NOT EXISTS "task_message_contexts" (
    "id" serial PRIMARY KEY NOT NULL,
    "task_message_id" integer NOT NULL,
    "type" text NOT NULL,
    "reference_id" text NOT NULL,
    "data" jsonb NOT NULL,
    "created_at" timestamptz DEFAULT now() NOT NULL
);

DO $$ BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'task_message_contexts_task_message_id_task_messages_id_fk'
    ) THEN
        ALTER TABLE "task_message_contexts"
            ADD CONSTRAINT "task_message_contexts_task_message_id_task_messages_id_fk"
            FOREIGN KEY ("task_message_id") REFERENCES "task_messages"("id") ON DELETE cascade ON UPDATE no action;
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS "task_message_contexts_message_idx" ON "task_message_contexts" USING btree ("task_message_id");
CREATE INDEX IF NOT EXISTS "task_message_contexts_reference_idx" ON "task_message_contexts" USING btree ("reference_id");

-- =========================================================================
-- Task message content blocks
-- =========================================================================
CREATE TABLE IF NOT EXISTS "task_message_content_blocks" (
    "id" serial PRIMARY KEY NOT NULL,
    "task_message_id" integer NOT NULL,
    "sort_order" integer DEFAULT 0 NOT NULL,
    "block_type" text NOT NULL,
    "show_in_ui" boolean DEFAULT true NOT NULL,
    "text_content" text,
    "tool_use_id" text,
    "tool_result_content" text,
    "tool_result_is_error" boolean,
    "image_media_type" text,
    "image_data" text,
    "created_at" timestamptz DEFAULT now() NOT NULL
);

DO $$ BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'task_message_content_blocks_task_message_id_task_messages_id_fk'
    ) THEN
        ALTER TABLE "task_message_content_blocks"
            ADD CONSTRAINT "task_message_content_blocks_task_message_id_task_messages_id_fk"
            FOREIGN KEY ("task_message_id") REFERENCES "task_messages"("id") ON DELETE cascade ON UPDATE no action;
    END IF;
END $$;

CREATE INDEX IF NOT EXISTS "task_message_content_blocks_message_idx" ON "task_message_content_blocks" USING btree ("task_message_id");

-- =========================================================================
-- Tags
-- =========================================================================
CREATE TABLE IF NOT EXISTS "tags" (
    "id" serial PRIMARY KEY NOT NULL,
    "tag_id" text NOT NULL,
    "name" text NOT NULL,
    "description" text,
    "created_at" timestamptz DEFAULT now() NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS "tags_tag_id_idx" ON "tags" USING btree ("tag_id");
CREATE UNIQUE INDEX IF NOT EXISTS "tags_name_idx" ON "tags" USING btree ("name");

-- =========================================================================
-- Task tags
-- =========================================================================
CREATE TABLE IF NOT EXISTS "task_tags" (
    "id" serial PRIMARY KEY NOT NULL,
    "task_id" text NOT NULL,
    "tag_id" text NOT NULL,
    "created_at" timestamptz DEFAULT now() NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS "task_tags_task_id_tag_id_idx" ON "task_tags" USING btree ("task_id", "tag_id");
CREATE INDEX IF NOT EXISTS "task_tags_tag_id_idx" ON "task_tags" USING btree ("tag_id");

-- =========================================================================
-- Commands
-- =========================================================================
CREATE TABLE IF NOT EXISTS "commands" (
    "name" text PRIMARY KEY NOT NULL,
    "description" text NOT NULL,
    "prompt" text NOT NULL,
    "conditions" jsonb,
    "icon" text,
    "variant" text,
    "active" boolean,
    "will_not_resume" boolean,
    "agent_mode" text,
    "scope" text NOT NULL DEFAULT 'global',
    "team_id" text NOT NULL DEFAULT 'system',
    "owner_user_id" text,
    "created_by" text DEFAULT 'system'
);

CREATE INDEX IF NOT EXISTS "commands_active_idx" ON "commands" USING btree ("active");

-- =========================================================================
-- File metadata
-- =========================================================================
CREATE TABLE IF NOT EXISTS "file_metadata" (
    "file_id" text PRIMARY KEY NOT NULL,
    "filename" text NOT NULL,
    "mime_type" text NOT NULL,
    "size" bigint NOT NULL,
    "scope" text NOT NULL DEFAULT 'global',
    "team_id" text NOT NULL DEFAULT 'system',
    "owner_user_id" text,
    "created_by" text DEFAULT 'system',
    "uploaded_at" timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS "file_metadata_scope_team_owner_idx"
    ON "file_metadata" ("scope", "team_id", "owner_user_id");

-- =========================================================================
-- Figma imports
-- =========================================================================
CREATE TABLE IF NOT EXISTS "figma_imports" (
    "import_id" text PRIMARY KEY NOT NULL,
    "file_id" text,
    "file_key" text,
    "node_id" text,
    "file_name" text,
    "node_name" text,
    "node_metadata" jsonb,
    "has_image" boolean,
    "include_image_in_prompt" boolean,
    "created_by" text,
    "created_at" timestamptz DEFAULT now()
);
