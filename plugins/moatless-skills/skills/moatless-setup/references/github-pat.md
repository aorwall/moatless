# GitHub access with an administrator PAT

Use this path for a Docker trial, a local evaluation, or a single-user installation. The PAT belongs to the signed-in Moatless administrator and is the quick repository-access path; it is not human-login configuration.

## Create the PAT

1. Keep the administrator signed in to Moatless, then open [GitHub Settings → Personal access tokens](https://github.com/settings/tokens/new) in the same browser.
2. Choose **Generate new token (classic)**, set a short expiration, and give it a purpose-specific name.
3. Select the `repo` scope. This is the scope the current Moatless product flow expects for private repository access.
4. If the repository belongs to an organization protected by SAML SSO, authorize the token for that organization from GitHub's token page after creating it.
5. Copy the token once and enter it in the authenticated Moatless browser at `/settings`, in the GitHub section. The form saves through `PUT /api/v1/settings/github/pat` and validates the token with GitHub before storing it.

The token value must never be pasted into chat, deployment environment variables, a `.env` file, a shell argument, a log, a screenshot, or a commit. Do not use `moat settings github-pat <token>`: the CLI accepts that form for compatibility, but a token in a command argument can enter shell history and process listings. Use the browser form instead. `moat settings github-status` is safe because it reads status and never accepts a token.

## Verify the connection

Run these authenticated, non-secret checks:

```bash
moat settings github-status --json
moat api GET /api/v1/settings/github/config
moat api GET /api/v1/auth/me
```

Require all of the following:

- the GitHub configuration reports `connected: true` and `tokenType: "pat"`;
- `/api/v1/auth/me` identifies the expected signed-in human;
- its `setup.requirements` contains `id: "github_token"` with `status: "satisfied"`.

The setup requirement proves that Moatless has a usable stored GitHub credential. It does not prove that the credential can reach the repository you intend to use, so continue with the repository check.

## Verify one user-scoped repository

Use a real repository that the PAT owner is allowed to read. List existing connections and choose one with `provider: "github"`, `scope: "user"`, and a GitHub remote:

```bash
moat repos list --json
```

If there is no suitable connection, create one without putting credentials in the URL. The owner and repository must be real and reachable by the PAT owner:

```bash
moat repos create \
  --name <repository-name> \
  --git-url https://github.com/<owner>/<repository>.git \
  --provider github \
  --scope user
```

Take the returned repository id and ask Moatless to verify the persisted remote with the credentials for its user scope:

```bash
moat api POST /api/v1/repositories/<repository-id>/verify-access
moat repos get <repository-id> --json
```

The repository response must report `accessStatus: "ok"`. Treat `not_found`, `forbidden`, and `error` as failed verification; repair the remote, PAT scope, organization SSO authorization, or repository ownership and repeat only the verification request.

Do not mark setup complete until the connection status, `github_token` requirement, and this real user-scoped repository check all pass. Bot-user creation, inbound GitHub PR webhooks, and global App installation are outside this PAT path.
