# GitHub access with a GitHub App

Use this path for a shared or production installation. Create a repository-access App separate from the OIDC application used for human login; do not reuse OIDC client settings, credentials, callbacks, or authorization state.

## Create and install the App

1. In GitHub, open [New GitHub App](https://github.com/settings/apps/new) and create a dedicated App for Moatless repository access.
2. Request the minimum repository permissions needed by the installation:
   - **Contents: Read and write**;
   - **Pull requests: Read and write**;
   - **Workflows: Read and write** only when agents must modify files under `.github/workflows`.

   Contents is required for HTTP Git access. GitHub's [permissions guidance](https://docs.github.com/en/apps/creating-github-apps/registering-a-github-app/choosing-permissions-for-a-github-app) describes the relationship between Contents and Git access and the additional Workflows permission.
3. Disable **Request user authorization (OAuth)** and disable webhooks for this repository-access workflow. Inbound GitHub PR webhooks are configured by the integrations workflow, not by this setup skill.
4. Generate a private key from the App settings and keep the downloaded PEM in a protected local file. Do not put the PEM in chat, a shell argument, logs, screenshots, a deployment value, or a commit.
5. Install the App on the intended personal account or organization. Prefer **Only select repositories** and select the real repository used for verification. GitHub's [installation guidance](https://docs.github.com/en/apps/using-github-apps/installing-a-github-app-from-a-third-party) explains that the installation owner chooses the repository set. Grant access to all repositories only when the user explicitly requests that scope.

## Register and bind it in Moatless

Use the signed-in administrator's browser at `/admin/integrations/github`:

1. Register the App with its stable Moatless key, GitHub App ID, and PEM signing key. The admin endpoint is `POST /api/v1/admin/settings/github/apps`; the server proves the key against GitHub before storing an encrypted copy and never returns the key.
2. Refresh installations after the GitHub installation is complete. The page's **Refresh installations** action reads `GET /api/v1/admin/settings/github/apps` and lists the accounts and installation ids GitHub reports.
3. Bind the selected App installation in **Global GitHub access**. The page uses `PUT /api/v1/admin/settings/github/installation`; binding is global so global repositories use that installation credential.

The safe CLI equivalents use only the PEM file path, never the PEM contents:

```bash
moat github-app register <app-key> \
  --app-id <github-app-id> \
  --key-file /secure/path/moatless-github-app.pem \
  --json
moat github-app list --json
moat api GET /api/v1/admin/settings/github/apps
moat api PUT /api/v1/admin/settings/github/installation \
  --body '{"githubAppKey":"<app-key>","installationId":<installation-id>}'
moat api GET /api/v1/admin/settings/github/installation
```

The CLI reads the PEM from `--key-file`, proves it, and prints only registration metadata and a key fingerprint. Keep the file outside the repository and remove or securely archive it according to the organization's key-retention policy after registration. Do not use `--verbose` for these commands.

Require the global binding response to contain `configured: true`, the expected `githubAppKey`, installation id, and account login. If the installation is missing, use the App's surfaced `installUrl`, install it on the intended account, then refresh the App list before binding. Do not bind a different account or silently broaden the installation.

Creating a Moatless bot user with `moat bot create --app-key <app-key>` is optional. It can provide an attribution identity, but it is not part of repository-access completion and must not block this workflow.

## Verify one global repository

After the binding is confirmed, use a real repository included in that installation. List existing connections and choose a GitHub repository with `scope: "global"`:

```bash
moat repos list --json
```

If there is no suitable connection, create one with the real GitHub remote and global scope:

```bash
moat repos create \
  --name <repository-name> \
  --git-url https://github.com/<owner>/<repository>.git \
  --provider github \
  --scope global
```

Verify the persisted remote with the bound installation credentials:

```bash
moat api POST /api/v1/repositories/<repository-id>/verify-access
moat repos get <repository-id> --json
moat api GET /api/v1/auth/me
```

Require `accessStatus: "ok"` for the global repository and `status: "satisfied"` for the `github_token` setup requirement in `/api/v1/auth/me`. Also require the global binding to remain configured. A registered App without an installation, a binding without repository access, or a successful API status without `accessStatus: "ok"` is incomplete.

Keep the App permissions and installation repository set narrow. Do not add Workflows permission unless it is needed, and do not select all repositories unless the user explicitly requests it. Leave webhook setup to the integrations workflow.
