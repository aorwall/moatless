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
3. Auto-generate `TASK_TOKEN_SECRET` if it's still the `your_*_here` placeholder.

The JSON report lists `keys_generated`, `keys_missing`, and `copied_assets`. On a fresh install, `keys_missing` will contain `CLAUDE_CODE_OAUTH_TOKEN` — that's expected. Use `env list` at any time to see the current state:

```bash
moat docker env list --json
```

`env list` reports each key with `status: set | placeholder | empty`. Secret keys (`CLAUDE_CODE_OAUTH_TOKEN`, `TASK_TOKEN_SECRET`) are masked as `**** (N chars)` — the actual value is never printed. Non-secret keys show their value so ports / domains / paths can be sanity-checked.

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

Then hand off to the `docker-testing` skill for task creation, preview debugging, and long-running ops.

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
| `CLAUDE_CODE_OAUTH_TOKEN` errors from backend | token expired or empty; refresh with `claude setup-token`, `moat docker init --overwrite` | §Claude token |
| Backend can't talk to Postgres | `moat docker status --json` → postgres endpoint detail; then `moat docker logs --service postgres` | §Database |
| First-boot migration failure | `moat docker logs --service backend --since 5m` | §Migrations |

See `references/troubleshooting.md` for expanded diagnostics.

## Additional Resources

### Reference files

- **`references/troubleshooting.md`** — port conflicts, Apple Silicon, socket permissions, Claude token refresh, DB wipe

### Assets

- **`assets/docker-compose.yml`** — symlink to the canonical compose file (used by `moat docker init --assets-from` when installing outside a repo checkout)
- **`assets/.env.example`** — symlink to the canonical env template
