-- =========================================================================
-- Channel Configurations
-- =========================================================================
CREATE TABLE IF NOT EXISTS "channel_configs" (
    "id" text PRIMARY KEY NOT NULL,
    "provider" text NOT NULL,
    "source_id" text NOT NULL,
    "source_name" text,
    "repository_id" text NOT NULL,
    "routing_mode" text NOT NULL,
    "prompt_template" text,
    "active" boolean NOT NULL DEFAULT true,
    "provider_config" jsonb,
    "ongoing_task_id" text,
    "created_by" text NOT NULL,
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "updated_at" timestamptz NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS "channel_configs_provider_source_idx"
    ON "channel_configs" ("provider", "source_id");
CREATE INDEX IF NOT EXISTS "channel_configs_repository_idx"
    ON "channel_configs" ("repository_id");
CREATE INDEX IF NOT EXISTS "channel_configs_active_idx"
    ON "channel_configs" ("active");

-- =========================================================================
-- Channel Threads
-- =========================================================================
CREATE TABLE IF NOT EXISTS "channel_threads" (
    "id" serial PRIMARY KEY NOT NULL,
    "config_id" text NOT NULL REFERENCES "channel_configs"("id"),
    "task_id" text NOT NULL,
    "external_thread_id" text NOT NULL,
    "properties" jsonb NOT NULL DEFAULT '{}',
    "last_message_at" timestamptz NOT NULL DEFAULT now(),
    "created_at" timestamptz NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS "channel_threads_task_idx" ON "channel_threads" ("task_id");
CREATE UNIQUE INDEX IF NOT EXISTS "channel_threads_config_thread_idx"
    ON "channel_threads" ("config_id", "external_thread_id");
