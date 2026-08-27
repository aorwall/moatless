-- =========================================================================
-- A Workspace may pin the GitHub credential its work presents
-- =========================================================================
-- A Workspace names the account every Task on it acts as on a git host, and a
-- pinned credential is one of the two ways it does so: `run_as_user_id` names a
-- person, and a pin here names a token directly. A pin is what reaches a
-- repository no identity in this deployment holds a credential for — one
-- outside the organisation the run-as user's own token is scoped to.
--
-- `moatless_identity::git_host_tokens::GitHostTokens` resolves the pin, so it
-- governs backend-side calls (opening a pull request, reading a repository) as
-- well as the `GITHUB_TOKEN` the sandbox clones and pushes with. Both present
-- the same account.
--
-- Values are envelope-encrypted like every other row in this table; the column
-- set is unchanged and only the legal provider set widens.

ALTER TABLE "workspace_credentials"
    DROP CONSTRAINT IF EXISTS "workspace_credentials_provider_chk",
    ADD CONSTRAINT "workspace_credentials_provider_chk"
        CHECK ("provider" IN ('github', 'gitlab', 'gitness', 'openai', 'anthropic'));
