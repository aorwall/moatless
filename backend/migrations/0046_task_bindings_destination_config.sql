-- Rename task_bindings.reply_target_config to destination_config.
--
-- The canonical task binding model uses destination_config for provider-specific
-- outbound delivery metadata. Keep this migration idempotent so fresh databases
-- that already created the column through 0043 still apply cleanly.

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_name = 'task_bindings'
          AND column_name = 'reply_target_config'
    ) AND NOT EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_name = 'task_bindings'
          AND column_name = 'destination_config'
    ) THEN
        ALTER TABLE "task_bindings"
            RENAME COLUMN "reply_target_config" TO "destination_config";
    END IF;
END $$;

ALTER TABLE "task_bindings"
    ADD COLUMN IF NOT EXISTS "destination_config" jsonb NOT NULL DEFAULT '{}'::jsonb;

UPDATE "task_bindings"
SET "destination_config" = COALESCE("destination_config", '{}'::jsonb)
WHERE "destination_config" IS NULL;
