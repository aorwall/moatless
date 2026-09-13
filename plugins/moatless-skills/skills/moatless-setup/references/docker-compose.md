# Docker Compose setup

Use this workflow for a laptop, evaluation, development, or single-host installation. Run it from a Moatless checkout or from the CLI-managed Compose directory selected by `moat docker`.

## Prepare

1. Check the host:

   ```bash
   moat docker preflight --json
   ```

   Continue only when every required check reports `ok: true`. Repair failures using each check's `hint`; do not bypass Docker, Compose, disk, port, or architecture failures.

2. Seed the Compose assets and generated secrets:

   ```bash
   moat docker init --non-interactive --json
   moat docker env list --json
   ```

   Confirm `TASK_TOKEN_SECRET` and `SECRETS_ENCRYPTION_KEY` are set. If `CLAUDE_CODE_OAUTH_TOKEN` is missing, have the user set it locally with `moat docker env set`. Do not ask for the value in chat or print `.env`.

3. Record `FRONTEND_PORT` from the non-secret environment listing. Default to `3000` only when it is unset.

## Ready for login

Preparation is complete when preflight passes, the Compose bundle exists, generated secrets are set, and the frontend port is known. Return to `SKILL.md`, configure one login mode, then resume below.

## Install and verify

1. Create or recreate the services from the configured env file:

   ```bash
   moat docker install --pull --wait
   moat docker status --json
   ```

   Use `moat docker install` after every auth env change; `docker restart` retains the old container environment.

2. Require `all_healthy: true`. If a service is unhealthy, inspect only that service with:

   ```bash
   moat docker logs --service <name> --follow
   ```

   Repair the reported cause and re-run install and status. Preserve the Postgres volume.

3. Verify the public frontend path:

   ```bash
   curl -fsS http://localhost:<frontend-port>/api/v1/auth/mode
   ```

   Open `http://localhost:<frontend-port>/login` for the real sign-in required by the main workflow.

The deployment checkpoint is complete when Compose reports healthy and the frontend serves the configured auth mode.
