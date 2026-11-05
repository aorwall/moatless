-- A Workspace no longer pins a GitHub credential: every GitHub call acts as an
-- identity's own connection, so the GitHub App can author what Tasks post.
-- Left in place, each row would be skipped with a warning on every sandbox
-- credential resolution.
DELETE FROM "workspace_credentials" WHERE "provider" = 'github';
