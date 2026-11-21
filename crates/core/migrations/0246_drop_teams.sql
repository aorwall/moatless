-- Teams are removed. Since 0097 a team is not a scope and grants no access,
-- and no code reads or writes `teams`, `team_members` or any `team_id` column.

DROP TABLE "team_members";

-- Dropping a column drops every index and constraint that names it, FKs onto
-- `teams` included, so the ones the code still relies on are recreated without
-- `team_id` afterwards.
ALTER TABLE "adapter_connections" DROP COLUMN "team_id";
ALTER TABLE "file_metadata" DROP COLUMN "team_id";
ALTER TABLE "loops" DROP COLUMN "team_id";
ALTER TABLE "plugins" DROP COLUMN "team_id";
ALTER TABLE "provider_tokens" DROP COLUMN "team_id";
ALTER TABLE "repositories" DROP COLUMN "team_id";
ALTER TABLE "secrets" DROP COLUMN "team_id";
ALTER TABLE "tasks" DROP COLUMN "team_id";
ALTER TABLE "workspaces" DROP COLUMN "team_id";
ALTER TABLE "api_keys" DROP COLUMN "team_ids";

DROP TABLE "teams";

-- `ON CONFLICT ON CONSTRAINT provider_tokens_scope_uq` in the identity DAOs.
ALTER TABLE "provider_tokens"
    ADD CONSTRAINT "provider_tokens_scope_uq"
    UNIQUE NULLS NOT DISTINCT ("user_id", "provider");

-- Same normalisation as 0096; only `team_id` leaves the key.
CREATE UNIQUE INDEX "repositories_active_owner_remote_url_uq"
    ON "repositories" (
        "scope",
        "owner_user_id",
        lower(regexp_replace(regexp_replace(regexp_replace(
            btrim("remote_url"), '/+$', ''), '\.git$', ''), '/+$', ''))
    ) NULLS NOT DISTINCT
    WHERE "remote_url" IS NOT NULL AND COALESCE("deleted", false) = false;

CREATE INDEX "repositories_scope_owner_idx" ON "repositories" ("scope", "owner_user_id");
CREATE INDEX "tasks_scope_owner_idx" ON "tasks" ("scope", "owner_user_id");
CREATE INDEX "file_metadata_scope_owner_idx" ON "file_metadata" ("scope", "owner_user_id");
CREATE INDEX "secrets_scope_owner_idx" ON "secrets" ("scope", "owner_user_id");

CREATE INDEX "idx_provider_tokens_codex_healthy_all_scopes"
    ON "provider_tokens" ("scope", "user_id")
    WHERE "provider" = 'codex' AND ("metadata" ->> 'refresh_failure') IS NULL;
