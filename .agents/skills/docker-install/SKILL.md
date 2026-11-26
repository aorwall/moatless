---
name: docker-install
description: "Install Moatless locally with moat docker. Use for first-time Docker Compose setup, preflight, initialization, credentials, and installation verification."
---

# Docker installation

Use the checkout's CLI or a matching release. The parser and installer in
[crates/cli/src/commands/docker](../../../crates/cli/src/commands/docker)
are the authority for flags and readiness checks.

1. Run `moat docker preflight --json`. Resolve failed checks using their hints;
   confirm Compose, ports, disk, and image architecture for this host.
2. Run `moat docker init --non-interactive --json`. Inspect `keys_missing` and
   `keys_generated`. Init seeds the Compose assets and generates missing system
   keys; it preserves existing values unless overwrite is explicitly selected.
3. Use `moat docker env list --json` to inspect masked credential status. The
   installer requires `CLAUDE_CODE_OAUTH_TOKEN`; obtain it with `claude setup-token`
   and enter it locally through `moat docker env set CLAUDE_CODE_OAUTH_TOKEN <token>`.
   Keep secret values out of chat and logs. Do not replace an existing
   `SECRETS_ENCRYPTION_KEY` to repair an agent login.
4. Run `moat docker install --pull --wait`, then `moat docker status --json`.
   Diagnose failed services with `moat docker logs --help` and the service name
   returned by status. Use configured URLs and ports; defaults are in
   [the Compose environment example](../../../docker-compose/.env.example).
5. Authenticate: `MOATLESS_API_URL=http://localhost:8080 moat login`, then
   `moat status`. Use the configured `BACKEND_PORT` if it differs. App API routes
   require authentication even on localhost.
6. Follow [manual smoke verification](../e2e-testing/references/manual-smoke.md).
   For a Script smoke, use a workspace that declares a Script, and verify
   the command's output. Docker publishes no Script ports, so report
   preview HTTP as unavailable. Report services and agent output separately.

Use [troubleshooting](references/troubleshooting.md) when a step fails.
For a running installation use [docker-testing](../docker-testing/SKILL.md).
Installer readiness alone does not prove agent credentials or preview behavior.
