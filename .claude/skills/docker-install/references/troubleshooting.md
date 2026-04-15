# Docker Install Troubleshooting

Expanded diagnostics for failure modes the CLI can't auto-remediate. SKILL.md has the quick-map; this file is loaded when a specific failure needs deeper investigation.

## Ports

Default ports: frontend `3000`, backend `8080`, postgres `5432`, redis `6379`, dozzle `9999`.

### Diagnosing a conflict

```bash
moat docker preflight --json | jq '.checks[] | select(.ok == false)'
```

Each failing port check names the service that needs it. To find the process holding a port:

```bash
# Linux
sudo ss -tlnp | grep :3000

# macOS
lsof -iTCP:3000 -sTCP:LISTEN -n -P
```

### Fixing without killing the other process

Edit `.env` in the compose work dir (the path `preflight` reports). Override keys:

```
FRONTEND_PORT=3001
BACKEND_PORT=8081
POSTGRES_PORT=5433
REDIS_PORT=6380
```

Re-run `moat docker install`. The compose file reads these env vars, so no further changes are needed.

Do **not** edit `docker-compose.yml` directly unless you also commit the change — user edits get clobbered on the next `moat docker init --overwrite`.

## Apple Silicon (arm64)

The published sandbox image is built for `linux/amd64`. On M-series Macs, Docker Desktop emulates x86 under Rosetta — it works, but Claude CLI subprocess startup is noticeably slower (~10–15 s vs ~2 s).

### Detect the mismatch

```bash
docker inspect aorwall/moatless-vibe-sandbox-js:latest --format '{{.Architecture}}'
# amd64 → emulated (slow, works)
# arm64 → native
```

### Build a native arm64 image

```bash
cd <repo-root>
./docker/build-sandbox.sh --local --only base
./docker/build-sandbox.sh --local --only js
```

Tag it to match `.env`'s `SANDBOX_IMAGE` (default `aorwall/moatless-vibe-sandbox-js:latest`). The build script cross-compiles the sandbox binary and installs the Claude CLI via npm for the host architecture.

After building:

```bash
moat docker uninstall
moat docker install
```

`uninstall` alone (no `--purge`) preserves Postgres data, so task history survives the rebuild.

## Claude token

### Expired or missing

Symptoms: backend logs show `401` or `authentication failed` from the Claude API; new tasks stall with no agent output.

```bash
# Refresh token
claude setup-token

# Write the new token and recreate the backend (restart alone is not enough —
# see §Stale-container-env below)
moat docker env set CLAUDE_CODE_OAUTH_TOKEN <new-token>
moat docker install
```

### Verifying the token is read by the container

```bash
docker exec moatless-backend sh -c 'echo ${CLAUDE_CODE_OAUTH_TOKEN:0:10}'
# Prints the first 10 chars if set; empty if not
```

If the env var shows up empty, the `.env` file isn't being picked up — check that `init` wrote to the same work dir compose is reading from (both reported by `moat docker status --json`).

## Database (Postgres)

### `pg_isready` fails in `status`

```bash
moat docker logs --service postgres --since 10m
```

Common causes:

- **Volume permission error** (Linux): the postgres image runs as UID 999 by default. If the named volume was created by a different user, it'll fail to write. Fix: `moat docker uninstall --purge` (wipes the volume) then `install`.
- **Port 5432 is bound by a host postgres**. The container starts but the published port fails; compose logs show `bind: address already in use`. Override `POSTGRES_PORT` in `.env`.

### Migration failure on first boot

Symptoms: backend container enters `CrashLoopBackOff`; logs show `migration ... failed`.

```bash
moat docker logs --service backend --since 5m | grep -i migrat
```

If a migration is wedged, clear DB state:

```bash
moat docker uninstall --purge    # destructive — drops postgres + redis volumes
moat docker install
```

This is safe for fresh installs but wipes task history. For production, take a backup first.

### Inspecting the DB directly

```bash
docker exec -it moatless-postgres psql -U moatless -d moatless
# Inside psql:
\dt                              # list tables
SELECT version FROM _sqlx_migrations ORDER BY version DESC LIMIT 5;
```

## Docker socket permissions

The backend container mounts `/var/run/docker.sock` to spawn sandbox containers. On Linux, the container runs as a user that must be in the `docker` group (GID shared with the host socket).

### Symptoms

Backend logs: `Got permission denied while trying to connect to the Docker daemon socket`.

### Fix (Linux)

```bash
# Check host socket group
ls -la /var/run/docker.sock
# → srw-rw---- 1 root docker 0 ... /var/run/docker.sock

# Confirm the socket is mounted
docker inspect moatless-backend --format '{{range .Mounts}}{{.Source}} -> {{.Destination}}{{"\n"}}{{end}}'
```

The compose file mounts the socket read-write; if the GID inside the container doesn't match the host `docker` group, adjust the `user:` or `group_add:` field in `docker-compose.yml`. This is environment-specific; the default config works on Docker Desktop (macOS/Windows) and most modern Linux distros.

### Docker Desktop (macOS)

`DATA_DIR` must be under a path Docker Desktop is configured to share. Defaults:

- `~/.moatless/data` → works (home is shared by default)
- `/opt/moatless/data` → requires adding `/opt/moatless` in Docker Desktop → Settings → Resources → File Sharing

Symptoms if not shared: `invalid mount config for type "bind": bind source path does not exist`.

## Stale state after upgrade

Symptoms: new image tag pulled, but frontend still shows old behavior, or backend logs reference fields that no longer exist.

```bash
moat docker uninstall              # stops containers, keeps volumes
moat docker install --pull         # pulls latest, restarts
```

If the schema changed in a backwards-incompatible way (rare, but happens on major tags):

```bash
moat docker uninstall --purge      # drops volumes; wipes task history
moat docker install --pull
```

Check the release notes (`git tag --sort=-creatordate | head -5` and `gh release view <tag>`) before purging.

## Stale-container-env

Containers freeze their environment at creation time. `docker restart <name>` reuses that snapshot — so `.env` edits (even via `moat docker env set`) don't take effect after a restart.

### Symptoms

- Edited `.env`, the new value is present in the file, but the backend still behaves as if the old value is in effect.
- Backend log still shows the old validation error (e.g. `SECRETS_ENCRYPTION_KEY must be base64 or raw 32 bytes`) after you just set a valid key.
- `docker exec moatless-backend sh -c 'printenv SECRETS_ENCRYPTION_KEY'` returns the old value.

### Fix

`docker compose up -d` detects env drift and recreates only the affected container:

```bash
moat docker install
```

A plain `docker restart` is never enough after `moat docker env set`. `moat docker install` is idempotent — safe to run anytime.

### Verification

```bash
docker inspect moatless-backend --format '{{range .Config.Env}}{{println .}}{{end}}' \
  | grep <KEY>
```

The printed value should match what's in `.env`.

## Agent-not-logged-in

### Symptoms

- Task runs, sandbox spawns, containers report healthy.
- Every agent response is a synthetic `Not logged in · Please run /login` — no tool calls, no file changes.
- `moat tasks get <id> --json | jq .totalCostUsd` stays at `0`.

This means the sandbox has a Claude CLI but no credentials. The backend failed to hand `CLAUDE_CODE_OAUTH_TOKEN` to the pod via `GetCredentials`.

### Root causes, in order of likelihood

1. **`SECRETS_ENCRYPTION_KEY` missing or malformed.** The backend accepts base64-decoding-to-32-bytes or raw 32 bytes. Anything else (e.g. 64-char hex) is rejected. Backend log shows:

    ```
    SECRETS_ENCRYPTION_KEY must be base64 or raw 32 bytes
    Failed to migrate env var to global secret ...
    Secret encryption is not configured
    ```

    Fix — clear the bad value, let `init` regenerate it as valid base64, then recreate the backend so the migration runs:

    ```bash
    moat docker env unset SECRETS_ENCRYPTION_KEY
    moat docker init --non-interactive
    moat docker install
    # Restart any task that errored under the old config:
    moat tasks restart <task-id>
    ```

2. **`CLAUDE_CODE_OAUTH_TOKEN` is expired.** See §Claude token above.

3. **Encryption configured, but the token never migrated.** On first boot with a valid `SECRETS_ENCRYPTION_KEY`, the backend logs:

    ```
    Migrated env var to global secret env_var=CLAUDE_CODE_OAUTH_TOKEN
    ```

    Absence of that line means the container started before the key was set (see §Stale-container-env) or the token itself is unset. Check `moat docker env list` for both `CLAUDE_CODE_OAUTH_TOKEN` and `SECRETS_ENCRYPTION_KEY` status = `set`, then `moat docker install` to recreate.

### Verification after fix

```bash
# New task, watch for a real turn (totalCostUsd > 0)
moat tasks create --repo Moatable --name "probe" --message "list files in src/"
moat tasks get <id> --json | jq '{status, agentState, totalCostUsd}'
```

A working install returns `totalCostUsd > 0` within ~60 s. See `SKILL.md` §Step 5 for the full smoke-test recipe.

## Cache dir cleanup

When running outside a repo checkout, `moat docker` uses `$HOME/.moatless/compose/` to hold `docker-compose.yml` + `.env`. To reset it completely:

```bash
moat docker uninstall --purge --purge-cache
```

This tears down containers, drops volumes, and removes the cache dir. Next install will re-seed from the skill assets.
