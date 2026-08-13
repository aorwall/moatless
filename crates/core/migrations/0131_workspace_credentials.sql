-- Provider credentials pinned to a Workspace.
--
-- A Workspace credential is the credential every Task running that Workspace
-- authenticates with, whoever the Task runs as. It outranks both the run-as
-- user's `provider_tokens` row and any global credential, so a Workspace can
-- be pointed at a dedicated machine account (a GitHub PAT, say) instead of
-- borrowing whichever human happened to start the Task.
--
-- Values are envelope-encrypted with the same cipher the `secrets` table and
-- `github_apps.private_key_ciphertext` use. `id` is the additional
-- authenticated data, exactly as `secrets.id` is for a secret, so ciphertext
-- cannot be moved between rows. Writes are audited into `secret_audit_events`
-- with `scope_type = 'workspace'` and `scope_id = workspace_id`; there is one
-- audit trail for credential material, not one per table.
--
-- Access control lives on the owning workspace row rather than here: setting a
-- credential requires control of the workspace, so this table carries no
-- scope/owner columns of its own to drift from it.

CREATE TABLE IF NOT EXISTS "workspace_credentials" (
    "id" text PRIMARY KEY NOT NULL,
    "workspace_id" text NOT NULL REFERENCES "workspaces"("id") ON DELETE CASCADE,
    -- Provider name, matching `provider_tokens.provider` where the two overlap.
    -- Only providers whose credential is a single static env var belong here;
    -- codex and the Claude OAuth blob are refreshed and written back per user,
    -- so a workspace-level copy of either would go stale unnoticed.
    "provider" text NOT NULL,
    "value_ciphertext" bytea NOT NULL,
    "encryption_nonce" bytea NOT NULL,
    "encrypted_data_key" bytea NOT NULL,
    "kms_key_id" text NOT NULL,
    "encryption_version" integer NOT NULL DEFAULT 1,
    "created_by" text NOT NULL,
    "updated_by" text NOT NULL,
    "created_at" timestamptz NOT NULL DEFAULT now(),
    "updated_at" timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT "workspace_credentials_provider_chk"
        CHECK ("provider" IN ('github', 'gitlab', 'gitness', 'openai', 'anthropic')),
    CONSTRAINT "workspace_credentials_encryption_version_chk"
        CHECK ("encryption_version" > 0)
);

-- One credential per provider per workspace: the override is a single value,
-- and a second row would make "which token does this workspace use" ambiguous.
CREATE UNIQUE INDEX IF NOT EXISTS "workspace_credentials_workspace_provider_uq"
    ON "workspace_credentials" ("workspace_id", "provider");
