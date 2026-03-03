# Docker Workflow

Build, run, and test in Docker Compose for local development and CI.

## Build Commands

All build commands use Taskfile (`task <command>`). Run from the project root.

| Component | Build Command | What It Does |
|-----------|--------------|-------------|
| Backend | `task build:backend` | Cross-compile Rust + Docker image (local load) |
| Frontend | `task build:frontend` | Vite build + nginx image |
| Sandbox | `task build:sandbox` | Rust sandbox binary + base/js images |
| Servers | `task build:servers` | files-server, preview-server images |

### Backend Build Details

The backend uses `cargo zigbuild` for cross-compilation (faster than QEMU):

```bash
# Build + load locally (native arch)
task build:backend

# Build for specific tag
task build:backend TAG=v1.0.0
```

## Start Docker Compose

### Full Stack (Docker Compose Only)

```bash
task compose:up
```

Starts: PostgreSQL, Redis, backend (port 8080), frontend (port 3000).

### Frontend Hot-Reload + Docker Compose Backend

```bash
task dev
```

Starts PostgreSQL, Redis, backend in Docker. Runs frontend dev server locally with Vite hot-reload, proxying API calls to the Docker backend via `VITE_REMOTE_API=http://localhost:8080`.

### Dev Docker (Full Build + Start)

```bash
./docker-compose/dev-docker.sh
```

Builds all images locally (sandbox included), starts everything, streams formatted logs.

## Rebuild After Changes

### Backend Only

```bash
task compose:refresh-backend
```

Rebuilds the backend Docker image and restarts only the backend container (PostgreSQL/Redis stay running).

### Full Restart

```bash
task compose:restart
```

Force-recreates all containers (re-reads env and config).

## Run E2E Tests

### All API Tests

```bash
GITHUB_PAT=ghp_xxx task test:api
```

### Single Test Chain

```bash
GITHUB_PAT=ghp_xxx task test:api:single -- task-crud
```

### With Debug Output

```bash
DEBUG_SSE=1 GITHUB_PAT=ghp_xxx task test:api
```

### View Test Report

```bash
npx playwright show-report test-results/api
```

## Check Logs

### Pretty-Printed Logs

```bash
task compose:logs
```

Uses the `docker-compose/logs.sh` script with color-coded output and log-level filtering.

### Raw Container Logs

```bash
# Backend
docker logs moatless-backend --tail 100
docker logs moatless-backend -f            # Follow

# Filter for errors (JSON log format)
docker logs moatless-backend 2>&1 | grep '"level":"ERROR"'

# Sandbox container (find by task ID)
docker ps --format "{{.Names}}" | grep moatless
docker logs moatless-sandbox-${TASK_ID}
```

### Service Status

```bash
task compose:ps
```

## Health Checks

```bash
# Run all health checks (PostgreSQL, Redis, backend, frontend, Docker socket)
task compose:verify

# Manual backend health check
curl -s http://localhost:8080/health
```

## Common Issues

### Backend Can't Reach Sandbox Containers

**Symptom**: Timeout errors when starting tasks

**Fix**: Ensure `SANDBOX_HOST=host.docker.internal` in docker-compose config. Backend creates sibling containers via Docker socket — they communicate via `host.docker.internal`.

### Database Connection Errors

**Check**: PostgreSQL is running and healthy:
```bash
docker exec moatless-postgres pg_isready
```

### Redis Connection Errors

**Check**: Redis is running:
```bash
docker exec moatless-redis redis-cli ping
```

### Image Not Found

```bash
# Pull latest images
docker compose -f docker-compose/docker-compose.yml pull
```

## Cleanup

```bash
# Stop all services
task compose:down

# Remove sandbox containers
docker rm -f $(docker ps -aq --filter "name=moatless-sandbox")

# Clean volumes (WARNING: deletes database data)
docker compose -f docker-compose/docker-compose.yml down -v
```

## Environment Variables

Key env vars in `.env` (or docker-compose environment):

| Variable | Required | Description |
|----------|----------|-------------|
| `CLAUDE_CODE_OAUTH_TOKEN` | Yes* | Claude Code authentication |
| `ANTHROPIC_API_KEY` | Yes* | Alternative to OAuth token |
| `TASK_TOKEN_SECRET` | Yes | JWT signing secret |
| `POSTGRES_PASSWORD` | Yes | Database password |
| `SANDBOX_IMAGE` | No | Sandbox Docker image (default: sandbox-js) |
| `DATA_DIR` | No | Host path for data (default: `$HOME/.moatless/data`) |

*One of `CLAUDE_CODE_OAUTH_TOKEN` or `ANTHROPIC_API_KEY` is required.
