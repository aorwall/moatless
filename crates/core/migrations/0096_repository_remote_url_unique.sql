-- Active repositories are unique by canonical remote URL within their access
-- owner. Matching trims surrounding whitespace, ignores case, and strips a
-- trailing slash and `.git`, mirroring `normalize_remote_url` in the backend.
--
-- Do not merge historical duplicates automatically: repositories can be
-- referenced by tasks and workspaces, so choosing a survivor requires an
-- explicit data decision.
DO $$
BEGIN
    IF EXISTS (
        SELECT 1
        FROM "repositories"
        WHERE "remote_url" IS NOT NULL
          AND COALESCE("deleted", false) = false
        GROUP BY
            "scope",
            "team_id",
            "owner_user_id",
            lower(
                regexp_replace(
                    regexp_replace(
                        regexp_replace(btrim("remote_url"), '/+$', ''),
                        '\.git$',
                        ''
                    ),
                    '/+$',
                    ''
                )
            )
        HAVING count(*) > 1
    ) THEN
        RAISE EXCEPTION
            'Cannot enforce repository URL uniqueness: active canonical URL duplicates exist within an access owner';
    END IF;
END $$;

CREATE UNIQUE INDEX "repositories_active_owner_remote_url_uq"
    ON "repositories" (
        "scope",
        "team_id",
        "owner_user_id",
        (
            lower(
                regexp_replace(
                    regexp_replace(
                        regexp_replace(btrim("remote_url"), '/+$', ''),
                        '\.git$',
                        ''
                    ),
                    '/+$',
                    ''
                )
            )
        )
    )
    NULLS NOT DISTINCT
    WHERE "remote_url" IS NOT NULL
      AND COALESCE("deleted", false) = false;
