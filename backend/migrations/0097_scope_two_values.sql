-- Collapse access-control scope to two values: 'global' | 'user'.
--
-- Teams leave access control. Team-scoped resources land with their owner
-- (COALESCE(owner_user_id, created_by)) so nothing becomes more visible than
-- it was; ownerless team-level settings in provider_tokens (GitHub App
-- installations, codex config, team credentials) become global, admin-managed.
-- Secrets move off the legacy scope_type/scope_id model onto the canonical
-- scope/owner_user_id columns. team_id columns and the team tables stay in
-- place, inert for access control.

-- 1) Entity tables: team rows land with their owner; ownerless rows -> global.
DO $$
DECLARE
    tbl text;
    n bigint;
BEGIN
    FOREACH tbl IN ARRAY ARRAY[
        'tasks', 'repositories', 'workspaces',
        'integration_connections', 'triggers', 'file_metadata'
    ] LOOP
        EXECUTE format(
            'UPDATE %I SET "scope" = ''user'', "owner_user_id" = COALESCE("owner_user_id", "created_by")
             WHERE "scope" = ''team'' AND COALESCE("owner_user_id", "created_by") IS NOT NULL', tbl);
        GET DIAGNOSTICS n = ROW_COUNT;
        RAISE NOTICE 'scope migration: %: team -> user (owner takes them): % row(s)', tbl, n;

        EXECUTE format(
            'UPDATE %I SET "scope" = ''global'', "owner_user_id" = NULL
             WHERE "scope" = ''team''', tbl);
        GET DIAGNOSTICS n = ROW_COUNT;
        IF n > 0 THEN
            RAISE WARNING 'scope migration: %: ownerless team rows -> global: % row(s)', tbl, n;
        END IF;

        -- Normalize user rows so the tightened owner constraints below hold.
        EXECUTE format(
            'UPDATE %I SET "owner_user_id" = "created_by"
             WHERE "scope" = ''user'' AND "owner_user_id" IS NULL AND "created_by" IS NOT NULL', tbl);
        EXECUTE format(
            'UPDATE %I SET "scope" = ''global''
             WHERE "scope" = ''user'' AND "owner_user_id" IS NULL', tbl);
        GET DIAGNOSTICS n = ROW_COUNT;
        IF n > 0 THEN
            RAISE WARNING 'scope migration: %: ownerless user rows -> global: % row(s)', tbl, n;
        END IF;
    END LOOP;
END $$;

-- 2) Secrets: canonical scope/owner_user_id become authoritative.
DO $$
DECLARE n bigint;
BEGIN
    UPDATE "secrets" SET "scope" = 'user', "owner_user_id" = "scope_id"
    WHERE "scope_type" = 'user';

    UPDATE "secrets" SET "scope" = 'global', "owner_user_id" = NULL
    WHERE "scope_type" = 'global';

    -- created_by is NOT NULL on secrets, so every team secret has an owner.
    UPDATE "secrets" SET "scope" = 'user', "owner_user_id" = "created_by"
    WHERE "scope_type" = 'team';
    GET DIAGNOSTICS n = ROW_COUNT;
    RAISE NOTICE 'scope migration: secrets: team -> user (owner takes them): % row(s)', n;

    -- Dedupe (scope, owner, key): the row that already shadowed wins — a
    -- user-scoped original over a converted team row — then newest updated_at.
    DELETE FROM "secrets" WHERE "id" IN (
        SELECT "id" FROM (
            SELECT "id", ROW_NUMBER() OVER (
                PARTITION BY "scope", COALESCE("owner_user_id", ''), "key"
                ORDER BY ("scope_type" = 'user') DESC, "updated_at" DESC, "id"
            ) AS rn
            FROM "secrets"
        ) ranked WHERE rn > 1
    );
    GET DIAGNOSTICS n = ROW_COUNT;
    IF n > 0 THEN
        RAISE WARNING 'scope migration: secrets: dropped duplicate row(s) after team collapse: %', n;
    END IF;
END $$;

DROP INDEX IF EXISTS "secrets_scope_key_uq";
DROP INDEX IF EXISTS "secrets_scope_idx";
ALTER TABLE "secrets" DROP CONSTRAINT IF EXISTS "secrets_scope_type_chk";
ALTER TABLE "secrets" DROP CONSTRAINT IF EXISTS "secrets_scope_id_chk";
ALTER TABLE "secrets" DROP COLUMN "scope_type";
ALTER TABLE "secrets" DROP COLUMN "scope_id";
CREATE UNIQUE INDEX "secrets_scope_key_uq"
    ON "secrets" ("scope", COALESCE("owner_user_id", ''), "key");

-- 3) provider_tokens: team-level settings are deployment infrastructure ->
--    global, admin-managed. When several teams configured the same provider,
--    keep the existing global row if present, else the most recently
--    connected team row.
DO $$
DECLARE n bigint;
BEGIN
    DELETE FROM "provider_tokens" t USING (
        SELECT ctid, ROW_NUMBER() OVER (
            PARTITION BY "provider"
            ORDER BY ("team_id" = 'system') DESC NULLS LAST,
                     "connected_at" DESC NULLS LAST
        ) AS rn
        FROM "provider_tokens" WHERE "user_id" IS NULL
    ) ranked
    WHERE t.ctid = ranked.ctid AND ranked.rn > 1;
    GET DIAGNOSTICS n = ROW_COUNT;
    IF n > 0 THEN
        RAISE WARNING 'scope migration: provider_tokens: dropped superseded team credential row(s): %', n;
    END IF;

    UPDATE "provider_tokens" SET "scope" = 'global', "owner_user_id" = NULL
    WHERE "user_id" IS NULL
      AND ("scope" IS DISTINCT FROM 'global' OR "owner_user_id" IS NOT NULL);
    GET DIAGNOSTICS n = ROW_COUNT;
    RAISE NOTICE 'scope migration: provider_tokens: team -> global: % row(s)', n;

    UPDATE "provider_tokens" SET "scope" = 'user', "owner_user_id" = "user_id"
    WHERE "user_id" IS NOT NULL
      AND ("scope" IS DISTINCT FROM 'user' OR "owner_user_id" IS DISTINCT FROM "user_id");
END $$;

-- team_id is now inert for access control and must not force a backing team
-- row (personal-team auto-creation is gone). provider_tokens.team_id also
-- participates in the upsert conflict key (UNIQUE NULLS NOT DISTINCT), so it
-- is normalized to NULL outright; elsewhere existing values stay as
-- provenance and new rows simply write NULL.
ALTER TABLE "tasks" ALTER COLUMN "team_id" DROP NOT NULL;
ALTER TABLE "repositories" ALTER COLUMN "team_id" DROP NOT NULL;
ALTER TABLE "secrets" ALTER COLUMN "team_id" DROP NOT NULL;
ALTER TABLE "file_metadata" ALTER COLUMN "team_id" DROP NOT NULL;
ALTER TABLE "provider_tokens" ALTER COLUMN "team_id" DROP NOT NULL;
UPDATE "provider_tokens" SET "team_id" = NULL;

-- 4) Tighten check constraints to the two-value model. Adding a constraint
--    validates every existing row, so this doubles as the assertion that no
--    team-scoped row survived the landing above.
ALTER TABLE "tasks" DROP CONSTRAINT IF EXISTS "tasks_owner_scope_chk";
ALTER TABLE "tasks" ADD CONSTRAINT "tasks_owner_scope_chk"
    CHECK (
        ("scope" = 'user' AND "owner_user_id" IS NOT NULL)
        OR ("scope" = 'global' AND "owner_user_id" IS NULL)
    );

ALTER TABLE "repositories" DROP CONSTRAINT IF EXISTS "repositories_scope_chk";
ALTER TABLE "repositories" ADD CONSTRAINT "repositories_scope_chk"
    CHECK ("scope" IN ('global', 'user'));
ALTER TABLE "repositories" DROP CONSTRAINT IF EXISTS "repositories_owner_scope_chk";
ALTER TABLE "repositories" ADD CONSTRAINT "repositories_owner_scope_chk"
    CHECK (
        ("scope" = 'user' AND "owner_user_id" IS NOT NULL)
        OR ("scope" = 'global' AND "owner_user_id" IS NULL)
    );

ALTER TABLE "file_metadata" DROP CONSTRAINT IF EXISTS "file_metadata_owner_scope_chk";
ALTER TABLE "file_metadata" ADD CONSTRAINT "file_metadata_owner_scope_chk"
    CHECK (
        ("scope" = 'user' AND "owner_user_id" IS NOT NULL)
        OR ("scope" = 'global' AND "owner_user_id" IS NULL)
    );

ALTER TABLE "workspaces" ADD CONSTRAINT "workspaces_owner_scope_chk"
    CHECK (
        ("scope" = 'user' AND "owner_user_id" IS NOT NULL)
        OR ("scope" = 'global' AND "owner_user_id" IS NULL)
    );

ALTER TABLE "integration_connections" ADD CONSTRAINT "integration_connections_owner_scope_chk"
    CHECK (
        ("scope" = 'user' AND "owner_user_id" IS NOT NULL)
        OR ("scope" = 'global' AND "owner_user_id" IS NULL)
    );

ALTER TABLE "triggers" ADD CONSTRAINT "triggers_owner_scope_chk"
    CHECK (
        ("scope" = 'user' AND "owner_user_id" IS NOT NULL)
        OR ("scope" = 'global' AND "owner_user_id" IS NULL)
    );

ALTER TABLE "secrets" ADD CONSTRAINT "secrets_owner_scope_chk"
    CHECK (
        ("scope" = 'user' AND "owner_user_id" IS NOT NULL)
        OR ("scope" = 'global' AND "owner_user_id" IS NULL)
    );

ALTER TABLE "provider_tokens" ADD CONSTRAINT "provider_tokens_owner_scope_chk"
    CHECK (
        ("scope" = 'user' AND "owner_user_id" IS NOT NULL)
        OR ("scope" = 'global' AND "owner_user_id" IS NULL)
    );
