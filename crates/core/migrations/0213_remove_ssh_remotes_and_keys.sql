-- SSH support is removed: no sandbox receives an SSH key, so every clone
-- authenticates over HTTPS.

-- Stored SSH private keys have no consumer left.
DELETE FROM "secrets" WHERE "kind" = 'ssh_key';

ALTER TABLE "secrets" DROP CONSTRAINT IF EXISTS "secrets_kind_chk";
ALTER TABLE "secrets" ADD CONSTRAINT "secrets_kind_chk"
    CHECK ("kind" IN ('env', 'provider_token', 'runtime_file', 'deployment', 'internal'));

-- `git@host:owner/name` and `ssh://[user@]host[:port]/owner/name` become
-- `https://host/owner/name`; an SSH port says nothing about the HTTPS one, so
-- it is dropped.
CREATE TEMP TABLE "ssh_remote_conversions" ON COMMIT DROP AS
SELECT
    "id",
    "scope",
    "team_id",
    "owner_user_id",
    COALESCE("deleted", false) AS "deleted",
    "remote_url" AS "ssh_url",
    CASE
        WHEN btrim("remote_url") ~ '^(git\+)?ssh://' THEN
            regexp_replace(
                btrim("remote_url"),
                '^(git\+)?ssh://([^@/]+@)?([^/:]+)(:[0-9]*)?/+',
                'https://\3/'
            )
        ELSE
            regexp_replace(btrim("remote_url"), '^[^@/:]+@([^/:]+):/*', 'https://\1/')
    END AS "https_url"
FROM "repositories"
WHERE "remote_url" IS NOT NULL
  AND (
      btrim("remote_url") ~ '^(git\+)?ssh://'
      OR btrim("remote_url") ~ '^[^@/:]+@[^/:]+:'
  );

-- Mirrors `normalize_remote_url` and the `repositories_active_owner_remote_url_uq`
-- key, so a converted row that would duplicate an active one is detected here
-- rather than failing the migration on the index.
CREATE FUNCTION pg_temp.normalized_remote_url(url text) RETURNS text AS $$
    SELECT lower(
        regexp_replace(
            regexp_replace(regexp_replace(btrim(url), '/+$', ''), '\.git$', ''),
            '/+$',
            ''
        )
    )
$$ LANGUAGE sql IMMUTABLE;

-- An active row whose HTTPS spelling another active row in the same owner
-- already holds keeps its SSH URL and is reported: merging the two is a data
-- decision, as 0096 records.
DELETE FROM "ssh_remote_conversions" c
WHERE NOT c."deleted"
  AND (
      EXISTS (
          SELECT 1
          FROM "repositories" r
          WHERE COALESCE(r."deleted", false) = false
            AND r."id" <> c."id"
            AND r."scope" = c."scope"
            AND r."team_id" IS NOT DISTINCT FROM c."team_id"
            AND r."owner_user_id" IS NOT DISTINCT FROM c."owner_user_id"
            AND pg_temp.normalized_remote_url(r."remote_url")
                = pg_temp.normalized_remote_url(c."https_url")
      )
      OR EXISTS (
          SELECT 1
          FROM "ssh_remote_conversions" o
          WHERE NOT o."deleted"
            AND o."id" < c."id"
            AND o."scope" = c."scope"
            AND o."team_id" IS NOT DISTINCT FROM c."team_id"
            AND o."owner_user_id" IS NOT DISTINCT FROM c."owner_user_id"
            AND pg_temp.normalized_remote_url(o."https_url")
                = pg_temp.normalized_remote_url(c."https_url")
      )
  );

DO $$
DECLARE
    skipped record;
BEGIN
    FOR skipped IN
        SELECT r."id", r."remote_url"
        FROM "repositories" r
        WHERE r."remote_url" IS NOT NULL
          AND COALESCE(r."deleted", false) = false
          AND (
              btrim(r."remote_url") ~ '^(git\+)?ssh://'
              OR btrim(r."remote_url") ~ '^[^@/:]+@[^/:]+:'
          )
          AND NOT EXISTS (SELECT 1 FROM "ssh_remote_conversions" c WHERE c."id" = r."id")
    LOOP
        RAISE NOTICE 'Repository % keeps SSH remote %: its HTTPS URL is already registered',
            skipped."id", skipped."remote_url";
    END LOOP;
END $$;

UPDATE "repositories" r
SET "remote_url" = c."https_url",
    "updated_at" = now()
FROM "ssh_remote_conversions" c
WHERE r."id" = c."id";

DROP FUNCTION pg_temp.normalized_remote_url(text);
