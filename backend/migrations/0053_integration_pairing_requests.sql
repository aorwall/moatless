-- Rename Telegram pairing storage out of the old channel namespace.

BEGIN;

ALTER TABLE IF EXISTS "channel_pairing_requests"
    RENAME TO "integration_pairing_requests";

DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_name = 'integration_pairing_requests'
          AND column_name = 'config_id'
    ) AND NOT EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_name = 'integration_pairing_requests'
          AND column_name = 'trigger_id'
    ) THEN
        ALTER TABLE "integration_pairing_requests"
            RENAME COLUMN "config_id" TO "trigger_id";
    END IF;

    IF EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_name = 'integration_pairing_requests'
          AND column_name = 'provider'
    ) AND NOT EXISTS (
        SELECT 1
        FROM information_schema.columns
        WHERE table_name = 'integration_pairing_requests'
          AND column_name = 'adapter_kind'
    ) THEN
        ALTER TABLE "integration_pairing_requests"
            RENAME COLUMN "provider" TO "adapter_kind";
    END IF;
END $$;

ALTER TABLE "integration_pairing_requests"
    DROP CONSTRAINT IF EXISTS "channel_pairing_requests_config_id_fkey";

ALTER TABLE "integration_pairing_requests"
    DROP CONSTRAINT IF EXISTS "integration_pairing_requests_trigger_id_fkey";

DELETE FROM "integration_pairing_requests" p
WHERE NOT EXISTS (
    SELECT 1 FROM "triggers" t WHERE t."id" = p."trigger_id"
);

ALTER TABLE "integration_pairing_requests"
    ADD CONSTRAINT "integration_pairing_requests_trigger_id_fkey"
    FOREIGN KEY ("trigger_id") REFERENCES "triggers"("id") ON DELETE CASCADE;

DROP INDEX IF EXISTS "idx_pairing_pending_user";
DROP INDEX IF EXISTS "idx_pairing_pending_code";

CREATE UNIQUE INDEX IF NOT EXISTS "integration_pairing_requests_pending_user_idx"
    ON "integration_pairing_requests" ("trigger_id", "adapter_kind", "provider_user_id")
    WHERE status = 'pending';

CREATE UNIQUE INDEX IF NOT EXISTS "integration_pairing_requests_pending_code_idx"
    ON "integration_pairing_requests" ("trigger_id", "code")
    WHERE status = 'pending';

COMMIT;
