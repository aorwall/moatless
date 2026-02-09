-- Migrate all timestamp columns from TIMESTAMP to TIMESTAMPTZ.
-- PostgreSQL assumes existing values are in the server timezone (UTC).

-- tasks
ALTER TABLE tasks ALTER COLUMN created_at_db TYPE timestamptz USING created_at_db AT TIME ZONE 'UTC';

-- repositories
ALTER TABLE repositories ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
ALTER TABLE repositories ALTER COLUMN updated_at TYPE timestamptz USING updated_at AT TIME ZONE 'UTC';

-- task_messages
ALTER TABLE task_messages ALTER COLUMN sent_at TYPE timestamptz USING sent_at AT TIME ZONE 'UTC';
ALTER TABLE task_messages ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';

-- task_message_contexts
ALTER TABLE task_message_contexts ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';

-- task_message_content_blocks
ALTER TABLE task_message_content_blocks ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';

-- tags
ALTER TABLE tags ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';

-- task_tags
ALTER TABLE task_tags ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';

-- task_events
ALTER TABLE task_events ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
ALTER TABLE task_events ALTER COLUMN delivered_at TYPE timestamptz USING delivered_at AT TIME ZONE 'UTC';

-- users
ALTER TABLE users ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
ALTER TABLE users ALTER COLUMN last_login_at TYPE timestamptz USING last_login_at AT TIME ZONE 'UTC';

-- sessions
ALTER TABLE sessions ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
ALTER TABLE sessions ALTER COLUMN expires_at TYPE timestamptz USING expires_at AT TIME ZONE 'UTC';

-- api_keys
ALTER TABLE api_keys ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
ALTER TABLE api_keys ALTER COLUMN expires_at TYPE timestamptz USING expires_at AT TIME ZONE 'UTC';
ALTER TABLE api_keys ALTER COLUMN last_used_at TYPE timestamptz USING last_used_at AT TIME ZONE 'UTC';

-- provider_tokens
ALTER TABLE provider_tokens ALTER COLUMN connected_at TYPE timestamptz USING connected_at AT TIME ZONE 'UTC';
ALTER TABLE provider_tokens ALTER COLUMN expires_at TYPE timestamptz USING expires_at AT TIME ZONE 'UTC';

-- oauth_states (if table exists from Drizzle migrations)
DO $$ BEGIN
    IF EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'oauth_states') THEN
        ALTER TABLE oauth_states ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';
        ALTER TABLE oauth_states ALTER COLUMN expires_at TYPE timestamptz USING expires_at AT TIME ZONE 'UTC';
    END IF;
END $$;

-- file_metadata
ALTER TABLE file_metadata ALTER COLUMN uploaded_at TYPE timestamptz USING uploaded_at AT TIME ZONE 'UTC';

-- figma_imports
ALTER TABLE figma_imports ALTER COLUMN created_at TYPE timestamptz USING created_at AT TIME ZONE 'UTC';

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
