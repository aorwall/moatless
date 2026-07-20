-- Allow `internal` as a secret kind. Internal secrets are backend-only values
-- (e.g. the OAuth token stashed behind a PAT override) and are excluded from
-- every sandbox delivery channel.

ALTER TABLE "secrets" DROP CONSTRAINT IF EXISTS "secrets_kind_chk";
ALTER TABLE "secrets" ADD CONSTRAINT "secrets_kind_chk"
    CHECK ("kind" IN ('env', 'provider_token', 'runtime_file', 'deployment', 'ssh_key', 'internal'));
