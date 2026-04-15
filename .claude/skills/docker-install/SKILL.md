---
name: Moatless Vibe Docker Installer
description: "This skill should be used when the user asks to 'install moatless locally', 'run moatless in docker', 'set up docker compose for moatless', 'bootstrap moatless dev env', 'start moatless on my laptop', or mentions first-time docker-compose setup. Drives `moat docker` subcommands (preflight → init → install → status) and hands off to the docker-testing skill once healthy."
---

# Moatless Vibe Docker Installer

Install Moatless Vibe locally on Docker Compose with a guided four-step flow. The CLI (`moat docker …`) does the actual work; this skill orchestrates ordering, interprets JSON output, and hands the running stack off to the `docker-testing` skill for day-to-day operations.

## When to trigger

Trigger on phrases like "install moatless locally", "run moatless in docker", "set up docker compose for moatless", "bootstrap moatless dev env", or any first-time install whose target is a single host running Docker.

Do **not** trigger for:
- An already-running stack (the user wants `docker-testing` — task creation, preview debugging)
- Kubernetes installs (use `k8s-install`)
- Production deploys (use `task deploy`)
- Upgrading an existing install (just re-run `moat docker install --pull`)

## Prerequisites

Before starting, confirm the user has:

- **Docker Engine 24+** with the **Compose v2** plugin. `docker compose version` must succeed — `docker-compose` (v1 standalone) is a fallback but not preferred.
- **~5 GB free disk** for images, Postgres data, and sandbox workspace.
- A **`CLAUDE_CODE_OAUTH_TOKEN`** — generated via `claude setup-token`. Stored in `.env` so sandbox containers can call the Claude CLI on behalf of tasks.
- Ports **3000, 8080, 5432, 6379, 9999** available on localhost (or willingness to override via `.env`).
- On **Apple Silicon**: optionally a locally-built arm64 sandbox image (see troubleshooting).

If any piece is missing, pause and ask before proceeding — do not guess.

## The four-step workflow

Execute these in order. Each step runs one `moat docker …` command and emits JSON the skill can parse (`--json`).

### Step 1 — Preflight

```bash
moat docker preflight --json
```

Reports host readiness. Interpret:

- Every `check.ok == true` → green light, continue.
- `check.ok == false` → surface the `check.hint` verbatim; do not proceed until fixed.
- `compose_command: null` → user is missing Compose v2. Offer to walk them through [Docker install](https://docs.docker.com/compose/install/).
- `compose_dir_source: "repo"` → user is inside a repo checkout; assets already on disk.
- `compose_dir_source: "cache"` → user is outside the repo; Step 2 will seed assets from this skill.
- Port `3000` / `8080` / etc. in use → surface the hint ("stop the process or set FRONTEND_PORT in .env").
- On Apple Silicon, note the sandbox-image-arch warning (see troubleshooting) but don't block on it.

### Step 2 — Init (seed files, set secrets via CLI)

**Always invoke init with `--non-interactive`.** Claude has no TTY for the masked password prompt. Use `moat docker env set` / `moat docker env list` to write and inspect `.env`; don't edit the file with an editor tool.

#### 2a. Seed the work dir

```bash
# When compose_dir_source was "repo":
moat docker init --non-interactive --json

# When compose_dir_source was "cache":
moat docker init --assets-from <this-skill>/assets --non-interactive --json
```

`<this-skill>` is the root directory of this skill — the path under which `SKILL.md` lives. Claude knows it because the skill has been loaded.

`init` does three things:
1. If the work dir is empty (cache-dir mode), copy `docker-compose.yml` + `.env.example` from `--assets-from`.
2. If `.env` is missing, copy `.env.example` → `.env`.
3. For each `your_*_here` placeholder, either auto-generate or flag as missing:
   - **`TASK_TOKEN_SECRET`** — 64-char hex, auto-generated.
   - **`SECRETS_ENCRYPTION_KEY`** — base64 of 32 random bytes, auto-generated. Not negotiable: without it the scoped-secrets subsystem stays off, `CLAUDE_CODE_OAUTH_TOKEN` never migrates to a stored secret, and the agent returns `Not logged in · Please run /login` on every message.
   - **`CLAUDE_CODE_OAUTH_TOKEN`** — the only key the user must supply. Ends up in `keys_missing` on a fresh non-interactive install; set it in 2b.

The JSON report lists `keys_generated`, `keys_missing`, and `copied_assets`. On a fresh install, `keys_generated` will contain `TASK_TOKEN_SECRET` and `SECRETS_ENCRYPTION_KEY`; `keys_missing` will contain `CLAUDE_CODE_OAUTH_TOKEN` — that's expected. Use `env list` at any time to see the current state:

```bash
moat docker env list --json
```

`env list` reports each key with `status: set | placeholder | empty`. Secret keys (`CLAUDE_CODE_OAUTH_TOKEN`, `TASK_TOKEN_SECRET`, `SECRETS_ENCRYPTION_KEY`) are masked as `**** (N chars)` — the actual value is never printed. Non-secret keys show their value so ports / domains / paths can be sanity-checked.

#### 2b. Set `CLAUDE_CODE_OAUTH_TOKEN`

Warn the user that the token will appear in the chat transcript before asking for it:

> Paste your `CLAUDE_CODE_OAUTH_TOKEN` (generate one with `claude setup-token` if needed). Note: it will appear in this chat's history — rotate it later if that's a concern.

Then write it to `.env` via the CLI — **do not** use the Edit/Write tool on `.env` directly:

```bash
moat docker env set CLAUDE_CODE_OAUTH_TOKEN <token>
```

The `env set` command:
- Creates `.env` from `.env.example` if it's missing.
- Upserts the key (no duplicate lines).
- Does not echo the value in its output (report says `Value: (secret, not echoed)`).

#### 2c. Verify

```bash
moat docker env list --json
```

Expect every required key to have `status: "set"`. If `CLAUDE_CODE_OAUTH_TOKEN` still shows `placeholder` or `empty`, the write didn't land — re-run `env set`. Only proceed to Step 3 once all required keys are set.

### Step 3 — Install

```bash
moat docker install --pull
```

Runs `docker compose pull` (refresh images) then `docker compose up -d`. For cautious users, drop `--pull` to skip the pre-fetch.

Add `--wait` to block until all services report healthy (120 s timeout). The JSON report includes `all_healthy: bool`, which can be `null` when not waited on.

If `up` fails, the CLI returns the streamed error. Most common failures:
- `bind: address already in use` → port conflict. Re-run preflight.
- `pull access denied` / `manifest not found` → wrong `APP_VERSION` or `SANDBOX_IMAGE`. Check `.env`.
- `invalid mount config` (macOS) → `DATA_DIR` not shared with Docker Desktop. Default `$HOME/.moatless/data` works.

### Step 4 — Status

```bash
moat docker status --json
```

`all_healthy: true` means every service is `running` and all endpoint probes (backend `/health`, frontend `/`, `pg_isready`, `redis PING`) pass.

If not healthy, the `services[]` array shows which container is wrong, and `endpoints[]` shows which probe failed. Suggest `moat docker logs --service <name> --follow` for the affected one.

Once healthy, give the user the URLs:
- **Frontend**: http://localhost:3000
- **Backend**: http://localhost:8080 (API at `/api/`, health at `/health`)
- **Dozzle** (log viewer): http://localhost:9999

**Do not hand off yet.** Healthy containers ≠ working agent, and a completed task ≠ a working preview. Proceed to Step 5 (including 5e's preview-reachability check) to verify end-to-end before declaring the install done.

### Step 5 — Verify end-to-end by creating a task

A healthy `status` means containers are up; it does not prove the agent can reach Claude. `SECRETS_ENCRYPTION_KEY` misconfiguration, expired OAuth tokens, or broken sandbox images all produce green `status` but `Not logged in · Please run /login` on every task message. Confirm the full path (backend → sandbox → credentials → Claude CLI → real file output) with a smoke-test task.

#### 5a. Register the local install as the default `moat` host

```bash
MOATLESS_API_URL=http://localhost:3000 moat login
```

`moat login` probes `/api/v1/auth/me`: a local install returns `providers: []` (no OAuth configured), so the CLI saves `http://localhost:3000` as a no-auth profile and makes it the default — no browser, no API key. Against a hosted install with OAuth configured, the same command kicks off the browser flow instead. Subsequent `moat` commands default to this host; no further `MOATLESS_API_URL` export needed. Confirm with:

```bash
moat status
```

Expect `API URL: http://localhost:3000` and `User: local-user (no auth)`.

#### 5b. Create a smoke-test task

```bash
moat repos list --json | jq -r '.[] | {id, name}'
moat tasks create --repo Moatable \
  --name "Smoke test: todo app" \
  --message "Create a simple todo app with add, complete, and delete. Use React and keep it minimal."
```

Capture the returned task ID and **give the user the task URL immediately** so they can watch progress in the UI while the agent runs:

```
http://localhost:3000/tasks/<task-id>
```

#### 5c. Poll until the agent settles

```bash
moat tasks get <task-id> --json | jq '{status, agentState, totalCostUsd}'
```

Typical completion: 60–90 s, cost ~$0.10. Interpret:

- `status: "open"` + `agentState: "idle"` + `totalCostUsd > 0` → agent completed a real turn. Continue to 5d.
- `status: "error"` → read `moat messages list <task-id>` for the failure. On fresh installs, the most common failure is `Not logged in · Please run /login` — that means `SECRETS_ENCRYPTION_KEY` isn't wired up. See `references/troubleshooting.md` §Agent-not-logged-in.
- `status: "running"` / `agentState: "running"` → still in flight; poll again in ~15 s.

#### 5d. Confirm the agent produced real output

A task that completed with `totalCostUsd > 0` but no file changes is a hallucinated turn, not a working install.

```bash
moat tasks files <task-id> tree
moat tasks files <task-id> read --path src/App.tsx
```

Only after seeing actual file content (`<App />` component with todo state, handlers, etc.) is step 5d satisfied. Continue to 5e.

#### 5e. Confirm the preview server is reachable

Files on disk aren't proof the generated app actually runs. The preview sidecar (Vite dev server, port 3000 inside the container → dynamic host port) must be `started` *and* respond to HTTP. Silent preview failures happen even when the agent completed successfully — a missed `startCommands` in the seed config, an arch-mismatched `node_modules/rollup` native binary on Apple Silicon, or a crash-looping container on exit 0 will all slip past Steps 5a–5d.

```bash
moat tasks status <task-id> --json | jq '.servers[] | {name, status, url, error}'
```

Expect the `preview` (or repo-configured default) server to report:

```json
{ "name": "preview", "status": "started", "url": "http://localhost:<dynamic-port>", "error": null }
```

Interpret:
- `status: "started"` + non-null `url` → continue to the HTTP check below.
- `status: "starting"` → vite is still booting (common right after task creation); re-poll after ~15 s. If it's still `starting` past ~60 s, treat it as a failure.
- `status: "failed"` with `error: "Crash loop (restarts: N, exit M)"` → the start command is broken. Read logs (`moat tasks logs <task-id> --server preview`), fix the repo's `servers[].startCommands`, then `moat tasks redeploy <task-id>` to recreate the container from the updated config. A plain `moat tasks restart` is not enough — it reuses the existing container's stale `Cmd`.
- `status: "failed"` with `error: "Server failed to become ready within timeout"` → the command runs but never binds the expected port. Check `moat tasks logs <task-id> --server preview` for the real error (common: `MODULE_NOT_FOUND` on rollup native bindings → fix via `moat tasks files <task-id> … rm -rf node_modules && bun install` or adjust the sandbox image arch).

Once the server reports `started`, hit the URL and confirm vite is actually serving HTML:

```bash
curl -sSf <url>/ | head -c 300
```

Expect an HTML document containing `<script type="module" src="/@vite/client"></script>` and `<div id="root">`. An empty response, connection refused, or non-200 status means the TCP port is open but the app isn't running — surface the failure rather than declaring success.

Only after both `status: "started"` *and* a successful HTML fetch is the install verified end-to-end. Report a one-line summary to the user — task URL, preview URL, elapsed time, cost, and "agent built todo app, preview reachable" — then hand off to the `docker-testing` skill for day-to-day ops.

For detailed `moat tasks` / `moat messages` / `moat tasks files` / `moat tasks logs` / `moat tasks redeploy` semantics, cross-reference the `moat-cli` skill.

## JSON output fields

Skill-parseable fields per command:

| Command | Key fields |
|---|---|
| `preflight` | `checks[]` (`.ok`, `.hint`), `compose_command`, `compose_dir`, `compose_dir_source` |
| `init` | `compose_dir`, `copied_assets[]`, `keys_generated[]`, `keys_prompted[]`, `keys_missing[]` |
| `install` | `up_exit_code`, `all_healthy`, `stderr` |
| `status` | `all_healthy`, `services[]` (`.name`, `.state`, `.health`), `endpoints[]` (`.ok`, `.detail`) |
| `uninstall` | `volumes_purged`, `cache_purged` |
| `env set` | `action` (`created`/`updated`/`unchanged`), `was_placeholder`, `is_secret`, `env_file` |
| `env unset` | `action` (`removed`/`not_present`), `key`, `env_file` |
| `env list` | `env_present`, `keys[]` (`.name`, `.status`, `.is_secret`, `.display`) |

## Safe retry

All subcommands are idempotent:
- `preflight` and `status` are read-only.
- `init` will **not** overwrite an existing `.env` unless `--overwrite` is set; it only fills in keys that are missing or still `your_*_here` placeholders.
- `install` is `docker compose up -d` — safe to re-run, brings the stack up to the desired state.
- `uninstall` without `--purge` leaves volumes (Postgres, Redis data) intact; `--purge` drops them (prompts confirmation in interactive mode).

If any step fails, re-run **that step** with the fix applied, not the whole flow.

## Failure quick-map

| Symptom | First thing to check | Deeper guide |
|---|---|---|
| Port conflict on `install` | `moat docker preflight --json` — which port, which process | `references/troubleshooting.md` §Ports |
| Sandbox image fails to start on M-series Mac | `docker inspect moatless-sandbox-js:latest` (check `Architecture`) | §Apple Silicon |
| `CLAUDE_CODE_OAUTH_TOKEN` errors from backend | token expired or empty; refresh with `claude setup-token`, `moat docker env set CLAUDE_CODE_OAUTH_TOKEN <new>`, `moat docker install` (recreates backend) | §Claude token |
| Agent says `Not logged in · Please run /login` in every task message | `SECRETS_ENCRYPTION_KEY` missing/malformed; `moat docker env unset SECRETS_ENCRYPTION_KEY && moat docker init --non-interactive && moat docker install` | §Agent-not-logged-in |
| `.env` edit doesn't take effect after `docker restart` | Container env is frozen at creation time; must run `moat docker install` to recreate | §Stale-container-env |
| Backend can't talk to Postgres | `moat docker status --json` → postgres endpoint detail; then `moat docker logs --service postgres` | §Database |
| First-boot migration failure | `moat docker logs --service backend --since 5m` | §Migrations |

See `references/troubleshooting.md` for expanded diagnostics.

## Additional Resources

### Reference files

- **`references/troubleshooting.md`** — port conflicts, Apple Silicon, socket permissions, Claude token refresh, DB wipe

### Assets

- **`assets/docker-compose.yml`** — symlink to the canonical compose file (used by `moat docker init --assets-from` when installing outside a repo checkout)
- **`assets/.env.example`** — symlink to the canonical env template
