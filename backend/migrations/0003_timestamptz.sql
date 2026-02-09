-- Migrate all timestamp columns from TIMESTAMP to TIMESTAMPTZ.
-- PostgreSQL accepts ALTER TYPE timestamptz on a column that is already
-- timestamptz (no-op), so this is safe for both fresh and existing databases.
-- Tables/columns that may not exist are wrapped in conditional blocks.

-- tasks
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'tasks' AND column_name = 'created_at_db') THEN
        ALTER TABLE tasks ALTER COLUMN created_at_db TYPE timestamptz USING created_at_db AT TIME ZONE 'UTC';
    END IF;
END $$;

-- repositories
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'repositories' AND column_name = 'created_at') THEN
        ALTER TABLE repositories ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
    END IF;
END $$;
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'repositories' AND column_name = 'updated_at') THEN
        ALTER TABLE repositories ALTER COLUMN updated_at TYPE timestamptz USING updated_at AT TIME ZONE 'UTC';
    END IF;
END $$;

-- task_messages
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'task_messages' AND column_name = 'sent_at') THEN
        ALTER TABLE task_messages ALTER COLUMN sent_at TYPE timestamptz USING sent_at AT TIME ZONE 'UTC';
    END IF;
END $$;
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'task_messages' AND column_name = 'created_at') THEN
        ALTER TABLE task_messages ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
    END IF;
END $$;

-- task_message_contexts
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'task_message_contexts' AND column_name = 'created_at') THEN
        ALTER TABLE task_message_contexts ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
    END IF;
END $$;

-- task_message_content_blocks
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'task_message_content_blocks' AND column_name = 'created_at') THEN
        ALTER TABLE task_message_content_blocks ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
    END IF;
END $$;

-- tags
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'tags' AND column_name = 'created_at') THEN
        ALTER TABLE tags ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
    END IF;
END $$;

-- task_tags
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'task_tags' AND column_name = 'created_at') THEN
        ALTER TABLE task_tags ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
    END IF;
END $$;

-- task_events (exists in Drizzle schema but not in 0001_schema.sql)
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'task_events') THEN
        ALTER TABLE task_events ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
        ALTER TABLE task_events ALTER COLUMN delivered_at TYPE timestamptz USING delivered_at AT TIME ZONE 'UTC';
    END IF;
END $$;

-- users
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'users' AND column_name = 'created_at') THEN
        ALTER TABLE users ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
    END IF;
END $$;
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'users' AND column_name = 'last_login_at') THEN
        ALTER TABLE users ALTER COLUMN last_login_at TYPE timestamptz USING last_login_at AT TIME ZONE 'UTC';
    END IF;
END $$;

-- sessions
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'sessions' AND column_name = 'created_at') THEN
        ALTER TABLE sessions ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
    END IF;
END $$;
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'sessions' AND column_name = 'expires_at') THEN
        ALTER TABLE sessions ALTER COLUMN expires_at TYPE timestamptz USING expires_at AT TIME ZONE 'UTC';
    END IF;
END $$;

-- api_keys
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'api_keys' AND column_name = 'created_at') THEN
        ALTER TABLE api_keys ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
    END IF;
END $$;
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'api_keys' AND column_name = 'expires_at') THEN
        ALTER TABLE api_keys ALTER COLUMN expires_at TYPE timestamptz USING expires_at AT TIME ZONE 'UTC';
    END IF;
END $$;
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'api_keys' AND column_name = 'last_used_at') THEN
        ALTER TABLE api_keys ALTER COLUMN last_used_at TYPE timestamptz USING last_used_at AT TIME ZONE 'UTC';
    END IF;
END $$;

-- provider_tokens (Drizzle schema may have different columns than 0001_schema.sql)
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'provider_tokens' AND column_name = 'connected_at') THEN
        ALTER TABLE provider_tokens ALTER COLUMN connected_at TYPE timestamptz USING connected_at AT TIME ZONE 'UTC';
    END IF;
END $$;
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'provider_tokens' AND column_name = 'expires_at') THEN
        ALTER TABLE provider_tokens ALTER COLUMN expires_at TYPE timestamptz USING expires_at AT TIME ZONE 'UTC';
    END IF;
END $$;
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'provider_tokens' AND column_name = 'created_at') THEN
        ALTER TABLE provider_tokens ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
    END IF;
END $$;
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'provider_tokens' AND column_name = 'updated_at') THEN
        ALTER TABLE provider_tokens ALTER COLUMN updated_at TYPE timestamptz USING updated_at AT TIME ZONE 'UTC';
    END IF;
END $$;

-- oauth_states (if table exists from Drizzle migrations)
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'oauth_states') THEN
        ALTER TABLE oauth_states ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
        ALTER TABLE oauth_states ALTER COLUMN expires_at TYPE timestamptz USING expires_at AT TIME ZONE 'UTC';
    END IF;
END $$;

-- file_metadata
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'file_metadata' AND column_name = 'uploaded_at') THEN
        ALTER TABLE file_metadata ALTER COLUMN uploaded_at TYPE timestamptz USING uploaded_at AT TIME ZONE 'UTC';
    END IF;
END $$;

-- figma_imports
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.columns
               WHERE table_name = 'figma_imports' AND column_name = 'created_at') THEN
        ALTER TABLE figma_imports ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
    END IF;
END $$;

-- plugin_marketplace (if table exists)
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'plugin_marketplace') THEN
        ALTER TABLE plugin_marketplace ALTER COLUMN last_synced_at TYPE timestamptz USING last_synced_at AT TIME ZONE 'UTC';
        ALTER TABLE plugin_marketplace ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
        ALTER TABLE plugin_marketplace ALTER COLUMN updated_at TYPE timestamptz USING updated_at AT TIME ZONE 'UTC';
    END IF;
END $$;

-- slack_channel_mappings (if table exists)
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'slack_channel_mappings') THEN
        ALTER TABLE slack_channel_mappings ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
        ALTER TABLE slack_channel_mappings ALTER COLUMN updated_at TYPE timestamptz USING updated_at AT TIME ZONE 'UTC';
    END IF;
END $$;
