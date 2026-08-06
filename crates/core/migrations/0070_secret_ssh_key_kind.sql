-- Allow `ssh_key` as a secret kind. SSH private keys are materialized to the
-- sandbox's ~/.ssh directory (never exposed as env vars) and used transparently
-- for git clone/push and ssh over SSH remotes.

ALTER TABLE "secrets" DROP CONSTRAINT IF EXISTS "secrets_kind_chk";
ALTER TABLE "secrets" ADD CONSTRAINT "secrets_kind_chk"
    CHECK ("kind" IN ('env', 'provider_token', 'runtime_file', 'deployment', 'ssh_key'));
