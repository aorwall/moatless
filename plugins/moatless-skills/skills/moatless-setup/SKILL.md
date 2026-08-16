---
name: moatless-setup
description: "Set up Moatless end to end on Docker Compose or Kubernetes, configure local email/password or OIDC human login, and verify GitHub repository access. Use for a new local, shared, or production installation and first-sign-in validation. Not for established-stack debugging, inbound GitHub webhook work, sandbox operations, or routine deployment updates."
---

# Moatless Setup

Bring a fresh Docker Compose or Kubernetes installation to a healthy deployment, one verified human login, and one verified GitHub repository.

## Workflow

1. Decide where Moatless will run before executing any setup command.

   Infer the target when the request or environment already makes it clear. Otherwise ask a ranked pick shaped by `/asking-questions` and `/writing-messages`:

   - Recommend Docker Compose for a laptop, evaluation, development, or a single-host installation.
   - Recommend Kubernetes for an existing cluster, a shared installation, or production operations that need ingress, persistent storage, and horizontal scaling.

   Done when exactly one target is selected. Do not initialize both targets while the decision is open.

2. Read the selected deployment reference completely, then execute only its **Prepare** section:

   - Docker Compose: `references/docker-compose.md`
   - Kubernetes: `references/kubernetes.md`

   Stop at its **Ready for login** checkpoint. Keep the selected reference available; its **Install and verify** section resumes after authentication is configured.

3. Decide how humans will sign in.

   Infer the mode when the user already specified it. Otherwise ask a ranked pick:

   - Recommend OIDC for shared or production installations. It supports automatic user provisioning and an optional exact domain allowlist.
   - Recommend local email/password for initial setup, isolated testing, or an explicitly local-only deployment.

   Then read and follow exactly one reference for the selected deployment target:

   - Local email/password: `references/email-password.md`
   - OIDC: `references/oidc.md`

   Complete every non-secret value and have the user enter secrets locally. Authentication configuration is done when the selected deployment's config contains one complete login mode and no secret was exposed in chat, output, logs, screenshots, or a committed file.

4. Return to the selected deployment reference and execute its **Install and verify** section. Re-run only the failed step after repairing its reported cause; preserve state established by successful earlier steps.

5. Verify one real browser sign-in through the installation's public frontend URL. Confirm `/api/v1/auth/mode` advertises the chosen mode and `/api/v1/auth/me` resolves the expected internal user after login. Do not begin GitHub access configuration before this succeeds.

6. Configure the `moat` CLI against the same public API before configuring GitHub.

   Use the public frontend origin as the API base URL because the CLI calls the installation's `/api/v1/*` routes. Do not include a trailing `/api/v1` in the value:

   ```bash
   MOATLESS_API_URL=<frontend-origin> moat status --json
   MOATLESS_API_URL=<frontend-origin> moat login --url <frontend-origin>
   MOATLESS_API_URL=<frontend-origin> moat whoami --json
   ```

   `moat login` probes `/api/v1/auth/mode`, then opens the browser for OIDC or prompts locally for the username and password for local auth. Never put a password or API key in a command, chat message, log, screenshot, or committed file. The standalone CLI saves its generated `mvk_` API key in `~/.config/moatless/credentials.json` with owner-only permissions; do not print that file. Keep `MOATLESS_API_URL=<frontend-origin>` on subsequent setup commands when more than one Moatless profile is configured.

   If `moat` is not installed, install it from the Moatless checkout according to the `moat-cli` skill, then repeat this checkpoint. It is complete only when `status` names the intended API URL and `whoami` resolves the expected signed-in user. Do not begin GitHub access configuration until this succeeds.

7. Choose the repository-access mode only after human login and CLI authentication succeed:

   - Recommend an administrator's short-lived classic PAT for a Docker trial or single-user evaluation.
   - Recommend a separate GitHub App for shared or production installations. Bind its selected-repository installation globally.

   Read and follow exactly one reference:

   - PAT: `references/github-pat.md`
   - GitHub App: `references/github-app.md`

   The chosen reference is complete only after the connection status and the `github_token` setup requirement are satisfied, and one real repository returns `accessStatus: ok` from the repository-access verification endpoint. Use the configured `moat` CLI profile for these checks. A successful credential save without repository verification is not completion.

## Boundaries

- Preserve existing databases, Redis data, object storage, persistent volumes, and namespaces. Require explicit authorization before any purge, volume deletion, or reinstall that discards data.
- Keep OIDC client secrets, local passwords, session keys, database credentials, registry tokens, agent credentials, CLI API keys, GitHub PATs, and GitHub App PEM keys out of chat, shell arguments, command output, logs, screenshots, deployment configuration, and commits. Enter secrets only through the intended masked prompt or authenticated browser form.
- Use one deployment reference per run. Never mix `moat docker` state with Helm or `moat k8s` state.
- This skill configures repository credentials and verifies repository access. Leave inbound GitHub PR webhook configuration to the integrations workflow.
- Hand an established Docker stack to `docker-testing` and an established Kubernetes installation to `kubernetes-testing` for debugging and routine operation.
