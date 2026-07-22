-- Internal-UUID user ids.
--
-- `users.id` and the three foreign-key user-id columns (`team_members.user_id`,
-- `user_provider_ids.user_id`, `user_credentials.user_id`) become native `uuid`.
-- The provider id (GitHub numeric id, `local:<login>`, `bot:<appkey>`, Auth0 sub)
-- is preserved in `users.provider_user_id` and as a `user_provider_ids` row, so
-- login resolves by `(provider, provider_user_id)`.
--
-- Polymorphic text columns (`owner_user_id`, `created_by`, `run_as_user_id`,
-- `sent_by`, `scope_id`, `team_id`, and the JSON `team_ids`) keep the uuid's text
-- form — they also carry sentinels (`system`) and prefixed values (`personal-<id>`)
-- that are not user ids. Personal team ids (`personal-<old-user-id>`) embed the old
-- id, so they and every reference to them are remapped to `personal-<uuid>`.

BEGIN;

-- ---------------------------------------------------------------------------
-- 1. Preserve the provider id before the id is replaced.
-- ---------------------------------------------------------------------------

ALTER TABLE "users" ADD COLUMN IF NOT EXISTS "provider_user_id" text;
UPDATE "users" SET "provider_user_id" = "id" WHERE "provider_user_id" IS NULL;

-- ---------------------------------------------------------------------------
-- 2. old -> new id maps (dropped at COMMIT).
-- ---------------------------------------------------------------------------

CREATE TEMP TABLE "_user_id_map" ON COMMIT DROP AS
    SELECT "id" AS old_id, gen_random_uuid()::text AS new_id FROM "users";
CREATE UNIQUE INDEX ON "_user_id_map" (old_id);

-- Personal teams only: their id/slug embed a user id. Real teams and the
-- `system` team keep their ids and are absent from this map.
CREATE TEMP TABLE "_team_id_map" ON COMMIT DROP AS
    SELECT 'personal-' || old_id AS old_id, 'personal-' || new_id AS new_id
    FROM "_user_id_map";
CREATE UNIQUE INDEX ON "_team_id_map" (old_id);

-- ---------------------------------------------------------------------------
-- 3. Drop every FK referencing users(id) or teams(id). The explicit re-add in
--    step 9 restores each one; verify the pg_constraint diff on a prod snapshot.
-- ---------------------------------------------------------------------------

DO $$
DECLARE
    r record;
BEGIN
    FOR r IN
        SELECT con.conname, cl.relname AS tbl
        FROM pg_constraint con
        JOIN pg_class cl ON cl.oid = con.conrelid
        JOIN pg_class ref ON ref.oid = con.confrelid
        WHERE con.contype = 'f' AND ref.relname IN ('users', 'teams')
    LOOP
        EXECUTE format('ALTER TABLE %I DROP CONSTRAINT %I', r.tbl, r.conname);
    END LOOP;
END $$;

-- ---------------------------------------------------------------------------
-- 4. Remap user ids. Columns are still text; sentinels (`system`) and non-user
--    values simply do not match the map and stay put.
-- ---------------------------------------------------------------------------

UPDATE "users" t               SET "id" = m.new_id           FROM "_user_id_map" m WHERE t."id" = m.old_id;
UPDATE "user_provider_ids" t   SET "user_id" = m.new_id      FROM "_user_id_map" m WHERE t."user_id" = m.old_id;
UPDATE "user_credentials" t    SET "user_id" = m.new_id      FROM "_user_id_map" m WHERE t."user_id" = m.old_id;
UPDATE "team_members" t        SET "user_id" = m.new_id      FROM "_user_id_map" m WHERE t."user_id" = m.old_id;
UPDATE "sessions" t            SET "user_id" = m.new_id      FROM "_user_id_map" m WHERE t."user_id" = m.old_id;
UPDATE "api_keys" t            SET "user_id" = m.new_id      FROM "_user_id_map" m WHERE t."user_id" = m.old_id;
UPDATE "provider_tokens" t     SET "user_id" = m.new_id      FROM "_user_id_map" m WHERE t."user_id" = m.old_id;
UPDATE "provider_tokens" t     SET "owner_user_id" = m.new_id FROM "_user_id_map" m WHERE t."owner_user_id" = m.old_id;
UPDATE "repositories" t        SET "owner_user_id" = m.new_id FROM "_user_id_map" m WHERE t."owner_user_id" = m.old_id;
UPDATE "repositories" t        SET "created_by" = m.new_id    FROM "_user_id_map" m WHERE t."created_by" = m.old_id;
UPDATE "tasks" t               SET "owner_user_id" = m.new_id FROM "_user_id_map" m WHERE t."owner_user_id" = m.old_id;
UPDATE "tasks" t               SET "created_by" = m.new_id    FROM "_user_id_map" m WHERE t."created_by" = m.old_id;
UPDATE "workspaces" t          SET "owner_user_id" = m.new_id FROM "_user_id_map" m WHERE t."owner_user_id" = m.old_id;
UPDATE "workspaces" t          SET "created_by" = m.new_id    FROM "_user_id_map" m WHERE t."created_by" = m.old_id;
UPDATE "file_metadata" t       SET "owner_user_id" = m.new_id FROM "_user_id_map" m WHERE t."owner_user_id" = m.old_id;
UPDATE "file_metadata" t       SET "created_by" = m.new_id    FROM "_user_id_map" m WHERE t."created_by" = m.old_id;
UPDATE "plugin_repos" t        SET "owner_user_id" = m.new_id FROM "_user_id_map" m WHERE t."owner_user_id" = m.old_id;
UPDATE "plugin_repos" t        SET "created_by" = m.new_id    FROM "_user_id_map" m WHERE t."created_by" = m.old_id;
UPDATE "feedback" t            SET "created_by" = m.new_id    FROM "_user_id_map" m WHERE t."created_by" = m.old_id;
UPDATE "task_messages" t       SET "sent_by" = m.new_id       FROM "_user_id_map" m WHERE t."sent_by" = m.old_id;
UPDATE "task_read_receipts" t  SET "user_id" = m.new_id       FROM "_user_id_map" m WHERE t."user_id" = m.old_id;
UPDATE "task_follows" t        SET "user_id" = m.new_id       FROM "_user_id_map" m WHERE t."user_id" = m.old_id;
UPDATE "integration_connections" t SET "owner_user_id" = m.new_id FROM "_user_id_map" m WHERE t."owner_user_id" = m.old_id;
UPDATE "integration_connections" t SET "created_by" = m.new_id    FROM "_user_id_map" m WHERE t."created_by" = m.old_id;
UPDATE "triggers" t            SET "owner_user_id" = m.new_id FROM "_user_id_map" m WHERE t."owner_user_id" = m.old_id;
UPDATE "triggers" t            SET "created_by" = m.new_id    FROM "_user_id_map" m WHERE t."created_by" = m.old_id;
UPDATE "trigger_rules" t       SET "run_as_user_id" = m.new_id FROM "_user_id_map" m WHERE t."run_as_user_id" = m.old_id;
UPDATE "secrets" t             SET "created_by" = m.new_id    FROM "_user_id_map" m WHERE t."created_by" = m.old_id;
UPDATE "secrets" t             SET "updated_by" = m.new_id    FROM "_user_id_map" m WHERE t."updated_by" = m.old_id;
UPDATE "secrets" t             SET "owner_user_id" = m.new_id FROM "_user_id_map" m WHERE t."owner_user_id" = m.old_id;
UPDATE "secrets" t             SET "scope_id" = m.new_id      FROM "_user_id_map" m WHERE t."scope_type" = 'user' AND t."scope_id" = m.old_id;
UPDATE "secret_audit_events" t SET "actor_id" = m.new_id      FROM "_user_id_map" m WHERE t."actor_id" = m.old_id;
UPDATE "secret_audit_events" t SET "scope_id" = m.new_id      FROM "_user_id_map" m WHERE t."scope_type" = 'user' AND t."scope_id" = m.old_id;

-- Personal team display names embed the old user id (`<id> Personal`).
UPDATE "teams" t SET "name" = m.new_id || ' Personal'
    FROM "_user_id_map" m WHERE t."name" = m.old_id || ' Personal';

-- ---------------------------------------------------------------------------
-- 5. Remap personal team ids. Slug becomes `personal-<uuid>` (= the new id),
--    matching what the application generates for a personal team.
-- ---------------------------------------------------------------------------

UPDATE "teams" t SET "slug" = m.new_id FROM "_team_id_map" m WHERE t."id" = m.old_id;
UPDATE "teams" t SET "id" = m.new_id   FROM "_team_id_map" m WHERE t."id" = m.old_id;

UPDATE "team_members" t            SET "team_id" = m.new_id FROM "_team_id_map" m WHERE t."team_id" = m.old_id;
UPDATE "provider_tokens" t         SET "team_id" = m.new_id FROM "_team_id_map" m WHERE t."team_id" = m.old_id;
UPDATE "repositories" t            SET "team_id" = m.new_id FROM "_team_id_map" m WHERE t."team_id" = m.old_id;
UPDATE "tasks" t                   SET "team_id" = m.new_id FROM "_team_id_map" m WHERE t."team_id" = m.old_id;
UPDATE "secrets" t                 SET "team_id" = m.new_id FROM "_team_id_map" m WHERE t."team_id" = m.old_id;
UPDATE "secrets" t                 SET "scope_id" = m.new_id FROM "_team_id_map" m WHERE t."scope_type" = 'team' AND t."scope_id" = m.old_id;
UPDATE "secret_audit_events" t     SET "scope_id" = m.new_id FROM "_team_id_map" m WHERE t."scope_type" = 'team' AND t."scope_id" = m.old_id;
UPDATE "workspaces" t              SET "team_id" = m.new_id FROM "_team_id_map" m WHERE t."team_id" = m.old_id;
UPDATE "file_metadata" t           SET "team_id" = m.new_id FROM "_team_id_map" m WHERE t."team_id" = m.old_id;
UPDATE "plugin_repos" t            SET "team_id" = m.new_id FROM "_team_id_map" m WHERE t."team_id" = m.old_id;
UPDATE "integration_connections" t SET "team_id" = m.new_id FROM "_team_id_map" m WHERE t."team_id" = m.old_id;
UPDATE "triggers" t                SET "team_id" = m.new_id FROM "_team_id_map" m WHERE t."team_id" = m.old_id;

-- ---------------------------------------------------------------------------
-- 6. Remap the JSON `team_ids` arrays (text holding a JSON array of team ids).
-- ---------------------------------------------------------------------------

UPDATE "sessions" s
SET "team_ids" = (
    SELECT COALESCE(jsonb_agg(COALESCE(m.new_id, a.elem) ORDER BY a.ord), '[]'::jsonb)::text
    FROM jsonb_array_elements_text(s."team_ids"::jsonb) WITH ORDINALITY AS a(elem, ord)
    LEFT JOIN "_team_id_map" m ON m.old_id = a.elem
)
WHERE s."team_ids" IS NOT NULL
  AND s."team_ids" <> ''
  AND jsonb_typeof(s."team_ids"::jsonb) = 'array';

UPDATE "api_keys" k
SET "team_ids" = (
    SELECT COALESCE(jsonb_agg(COALESCE(m.new_id, a.elem) ORDER BY a.ord), '[]'::jsonb)::text
    FROM jsonb_array_elements_text(k."team_ids"::jsonb) WITH ORDINALITY AS a(elem, ord)
    LEFT JOIN "_team_id_map" m ON m.old_id = a.elem
)
WHERE k."team_ids" IS NOT NULL
  AND k."team_ids" <> ''
  AND jsonb_typeof(k."team_ids"::jsonb) = 'array';

-- ---------------------------------------------------------------------------
-- 7. Backfill provider identities for login. `github` and `github_app` collapse
--    to a single `github` lookup key.
-- ---------------------------------------------------------------------------

INSERT INTO "user_provider_ids" ("user_id", "provider", "provider_user_id", "created_at")
SELECT
    "id",
    CASE WHEN "provider" IN ('github', 'github_app') THEN 'github' ELSE "provider" END,
    "provider_user_id",
    now()
FROM "users"
WHERE "provider_user_id" IS NOT NULL
ON CONFLICT ("user_id", "provider") DO NOTHING;

-- ---------------------------------------------------------------------------
-- 8. Convert column types. Values already hold uuid text; the USING cast
--    validates them. Polymorphic reference columns stay text.
-- ---------------------------------------------------------------------------

ALTER TABLE "users"             ALTER COLUMN "id"      TYPE uuid USING "id"::uuid;
ALTER TABLE "team_members"      ALTER COLUMN "user_id" TYPE uuid USING "user_id"::uuid;
ALTER TABLE "user_provider_ids" ALTER COLUMN "user_id" TYPE uuid USING "user_id"::uuid;
ALTER TABLE "user_credentials"  ALTER COLUMN "user_id" TYPE uuid USING "user_id"::uuid;

-- ---------------------------------------------------------------------------
-- 9. Re-add the dropped FKs. The four CASCADE constraints are load-bearing.
-- ---------------------------------------------------------------------------

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'team_members_user_id_fkey') THEN
        ALTER TABLE "team_members" ADD CONSTRAINT "team_members_user_id_fkey"
            FOREIGN KEY ("user_id") REFERENCES "users"("id") ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'user_provider_ids_user_id_fkey') THEN
        ALTER TABLE "user_provider_ids" ADD CONSTRAINT "user_provider_ids_user_id_fkey"
            FOREIGN KEY ("user_id") REFERENCES "users"("id") ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'user_credentials_user_id_fkey') THEN
        ALTER TABLE "user_credentials" ADD CONSTRAINT "user_credentials_user_id_fkey"
            FOREIGN KEY ("user_id") REFERENCES "users"("id") ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'team_members_team_id_fkey') THEN
        ALTER TABLE "team_members" ADD CONSTRAINT "team_members_team_id_fkey"
            FOREIGN KEY ("team_id") REFERENCES "teams"("id") ON DELETE CASCADE;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'provider_tokens_team_id_fkey') THEN
        ALTER TABLE "provider_tokens" ADD CONSTRAINT "provider_tokens_team_id_fkey"
            FOREIGN KEY ("team_id") REFERENCES "teams"("id");
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'repositories_team_id_fkey') THEN
        ALTER TABLE "repositories" ADD CONSTRAINT "repositories_team_id_fkey"
            FOREIGN KEY ("team_id") REFERENCES "teams"("id");
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'integration_connections_team_id_fkey') THEN
        ALTER TABLE "integration_connections" ADD CONSTRAINT "integration_connections_team_id_fkey"
            FOREIGN KEY ("team_id") REFERENCES "teams"("id");
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'triggers_team_id_fkey') THEN
        ALTER TABLE "triggers" ADD CONSTRAINT "triggers_team_id_fkey"
            FOREIGN KEY ("team_id") REFERENCES "teams"("id");
    END IF;
END $$;

COMMIT;
