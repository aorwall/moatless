# Docker Compose Deployment Guide

This guide covers deploying Moatless Vibe using Docker Compose with PostgreSQL and Docker-based sandbox provisioning.

## Table of Contents

1. [Quick Start](#quick-start)
2. [Architecture](#architecture)
4. [Configuration](#configuration)
5. [Common Operations](#common-operations)
6. [Troubleshooting](#troubleshooting)
7. [Production Deployment](#production-deployment)

## Quick Start

To run the current Rust backend behind the T3 Code web app, use the dedicated
[T3 Code local-development runbook](../docs/runbooks/t3-code-local-development.md).
It uses an additive Compose overlay and leaves this published-image workflow
unchanged.

### Prerequisites

- Docker 20.10+ and Docker Compose v2+ installed
- At least 4GB RAM available
- Ports 3000, 5432, 6379, and 8080 available

### Setup

1. **Clone the repository**
   ```bash
   git clone https://github.com/aorwall/moatless-vibe.git
   cd moatless-vibe
   ```

2. **Run setup script**
   ```bash
   chmod +x docker-compose/docker-compose-setup.sh
   docker-compose/docker-compose-setup.sh
   ```

   The script will:
   - Check Docker prerequisites
   - Create `.env` from template
   - Prompt you to configure required values
   - Pull Docker images
   - Start all services (including frontend)

3. **Configure required values in `.env`**
   - `CLAUDE_CODE_OAUTH_TOKEN` - Get from https://claude.ai/settings/code
   - `TASK_TOKEN_SECRET` - Generate with: `openssl rand -hex 32`
   - `POSTGRES_PASSWORD` - Change from default

4. **Verify deployment**
   ```bash
   docker-compose/docker-compose-verify.sh
   ```

5. **Access the application**
   - Frontend: http://localhost:3000
   - Backend API: http://localhost:8080
   - API Health: http://localhost:8080/health

## Architecture

### System Overview

```
┌─────────────────┐
│  Frontend       │
│  (nginx)        │
│  :3000          │
│                 │
│  / → SPA files  │
│  /api/ → proxy ─┼──┐
└─────────────────┘  │
                     ▼
              ┌─────────────┐
              │   Backend   │
              │ (Rust/Axum) │
              │    :8080    │
              └──────┬──────┘
                     │
              ┌──────┴──────────────────┐
              ▼                         ▼
        ┌──────────┐            ┌──────────────┐
        │PostgreSQL│            │Docker Socket │
        │  :5432   │            │ (sibling     │
        └──────────┘            │  containers) │
                                └──────────────┘
                                           │
                                    ┌──────▼──────┐
                                    │  Sandbox    │
                                    │ Containers  │
                                    └─────────────┘
```

### Services

#### Frontend
- **Image**: `aorwall/moatless-frontend:latest` (nginx serving React SPA)
- **Purpose**: Serves the React application and proxies `/api/` requests to the backend
- **Port**: 3000 (configurable via `FRONTEND_PORT`)
- **Config**: Custom `nginx.conf` mounted to handle SPA routing + API proxy

#### Backend
- **Image**: `aorwall/moatless-backend:latest` (Rust/Axum)
- **Purpose**: API server, sandbox orchestration, database migrations
- **Port**: 8080 (configurable via `BACKEND_PORT`)
- **Key Features**:
  - Docker socket access for sandbox provisioning
  - Automatic database migrations on startup
  - Event streaming from the Postgres task-event feed
  - gRPC communication with sandbox containers

#### PostgreSQL
- **Image**: postgres:17-alpine
- **Purpose**: Primary database for tasks, messages, events, repositories, and the event feed SSE reads from
- **Port**: 5432 (configurable)
- **Data**: Persisted in `postgres_data` volume

#### Dozzle
- **Image**: amir20/dozzle:latest
- **Purpose**: Log viewer for all moatless containers
- **Port**: 9999 (configurable)

### Network Flow

1. **Browser → Frontend (nginx)**: HTTP requests to `localhost:3000`
2. **Frontend → Backend**: nginx proxies `/api/*` to `backend:8080`
3. **Backend → PostgreSQL**: Database queries via service name `postgres:5432`
4. **Backend → Docker Socket**: Creates sibling sandbox containers via `/var/run/docker.sock`
5. **Sandbox → Backend**: gRPC calls via `moatless-backend:50051`

### Volume Mounts

| Volume | Container Path | Purpose |
|--------|---------------|---------|
| `postgres_data` | `/var/lib/postgresql/data` | PostgreSQL database files |
| `../data` | `/data` | Shared data for backend and sandboxes |
| Docker socket | `/var/run/docker.sock` | Container management (read-only) |
| `nginx.conf` | `/etc/nginx/conf.d/default.conf` | Frontend nginx configuration |

### Sandbox Workspace Provisioning

Each task gets its own isolated Docker sandbox container:

```
┌─────────────────────────────────────────────────────┐
│ Backend Container                                    │
│                                                      │
│  ┌──────────────────────────────────────────────┐  │
│  │ Docker Service (via /var/run/docker.sock)   │  │
│  └────────────────┬─────────────────────────────┘  │
│                   │ Creates sibling containers      │
│                   ▼                                  │
└───────────────────┼──────────────────────────────────┘
                    │
     ┌──────────────┴──────────────┐
     ▼                             ▼
┌─────────────────┐       ┌─────────────────┐
│ Sandbox Task 1  │       │ Sandbox Task 2  │
│                 │       │                 │
│ /opt/moatless/  │       │ /opt/moatless/  │
│ ├─ repo/        │       │ ├─ repo/        │
│ │  (git clone)  │       │ │  (git clone)  │
│ └─ data/ ←──────┼───┐   │ └─ data/ ←──────┼───┐
│    (per-task)   │   │   │    (per-task)   │   │
└─────────────────┘   │   └─────────────────┘   │
                      │                           │
                      │   Host Data Directory     │
                      │   ./data/tasks/           │
                      ├──▶ {task-id-1}/           │
                      └──▶ {task-id-2}/           │
```

The sandbox provider is set to `docker` automatically in the compose file (`SANDBOX_PROVIDER: docker`). No additional configuration is needed.

## Configuration

### Required Environment Variables

Edit `.env` and set these required values:

#### 1. Claude Authentication (REQUIRED)
```bash
CLAUDE_CODE_OAUTH_TOKEN=your_token_here
```
Get from: https://claude.ai/settings/code

Alternatively, use Anthropic API key:
```bash
ANTHROPIC_API_KEY=sk-ant-...
```

#### 2. Task Token Secret (REQUIRED)
```bash
TASK_TOKEN_SECRET=$(openssl rand -hex 32)
```

#### 3. Database Password
```bash
POSTGRES_PASSWORD=your_secure_password_here
```
Change from default for security.

### Optional Configuration

#### Authentication Provider

Local username/password auth (the default — leave `AUTH_PROVIDER` unset or
`local`):
```bash
AUTH_PROVIDER=local
# Signs session cookies. Without it the backend falls back to TASK_TOKEN_SECRET,
# so rotating that would log everyone out.
SESSION_JWT_SECRET=$(openssl rand -hex 32)
# Bootstrap admin (minimum 12 characters). Omit it and a random password is
# generated on first boot and logged once at WARN:
#   docker compose logs backend | grep "generated a bootstrap admin password"
ADMIN_USERNAME=admin
ADMIN_PASSWORD=choose-a-strong-password
```

Sign in at `/login`. There is no unauthenticated mode — every API route
requires a session cookie or an `mvk_` API key.

Or enable GitHub App login:
```bash
AUTH_PROVIDER=github_app
GITHUB_APP_CLIENT_ID=your_client_id
GITHUB_APP_CLIENT_SECRET=your_client_secret
GITHUB_APP_CALLBACK_URL=http://localhost:8080/api/v1/auth/github-app/callback
```

Create a GitHub App at: https://github.com/settings/apps/new

Or enable Auth0:
```bash
AUTH_PROVIDER=auth0
AUTH0_DOMAIN=your-tenant.auth0.com
AUTH0_CLIENT_ID=your_client_id
AUTH0_CLIENT_SECRET=your_client_secret
AUTH0_CALLBACK_URL=http://localhost:8080/api/v1/auth/auth0/callback
```

#### Sandbox Image

Choose sandbox environment:

**JavaScript/TypeScript only** (default, 2GB):
```bash
SANDBOX_IMAGE=aorwall/moatless-vibe-sandbox-js:latest
```

**Full stack** (includes Python, Rust, Go, Java, 5-8GB):
```bash
SANDBOX_IMAGE=aorwall/moatless-vibe-sandbox:latest
```

#### Port Configuration

Customize ports to avoid conflicts:
```bash
FRONTEND_PORT=3000
BACKEND_PORT=8080
POSTGRES_PORT=5432
```

### Environment Variable Reference

See `.env.example` for complete list with descriptions.

## Common Operations

### Start Services

```bash
docker compose -f docker-compose/docker-compose.yml up -d
```

### View Logs

All services:
```bash
docker compose -f docker-compose/docker-compose.yml logs -f
```

Specific service:
```bash
docker compose -f docker-compose/docker-compose.yml logs -f backend
docker compose -f docker-compose/docker-compose.yml logs -f frontend
```

Or use the log viewer script:
```bash
docker-compose/logs.sh
```

Or use Dozzle at http://localhost:9999.

### Stop Services

```bash
docker compose -f docker-compose/docker-compose.yml down
```

### Update to Latest Version

```bash
docker compose -f docker-compose/docker-compose.yml pull
docker compose -f docker-compose/docker-compose.yml up -d
```

### View Service Status

```bash
docker compose -f docker-compose/docker-compose.yml ps
```

### View Sandbox Containers

List running sandboxes:
```bash
docker ps --filter "label=created-by=moatless"
```

View sandbox logs:
```bash
docker logs <container-name>
```

### Clean Up Sandbox Containers

Remove all sandbox containers:
```bash
docker rm -f $(docker ps -aq --filter "label=created-by=moatless")
```

### Reset Database

**Warning**: This deletes all data!

```bash
docker compose -f docker-compose/docker-compose.yml \
   down -v
docker compose -f docker-compose/docker-compose.yml up -d
```

### Backup Database

```bash
docker compose -f docker-compose/docker-compose.yml \
  exec postgres pg_dump -U moatless moatless > backup.sql
```

Restore:
```bash
cat backup.sql | docker compose -f docker-compose/docker-compose.yml \
  exec -T postgres psql -U moatless -d moatless
```

## Building Images Locally

### Frontend Image
Build it from the `moatless-legacy-frontend` repository:
```bash
docker/build-app.sh --local
```

### Sandbox Images
```bash
docker/build-sandbox.sh --local
```

### Backend Image
```bash
docker build -t aorwall/moatless-backend:latest -f docker/Dockerfile.backend .
```

The `dev-docker.sh` script builds the backend image automatically.

## Troubleshooting

### Backend Can't Create Containers

**Symptom**: "Cannot connect to Docker daemon" error in backend logs

**Solution**:
1. Ensure Docker socket is mounted in `docker-compose.yml`
2. Check backend service has volume: `/var/run/docker.sock:/var/run/docker.sock:ro`
3. Restart backend: `docker compose -f docker-compose/docker-compose.yml restart backend`

### Sandboxes Can't Reach Backend

**Symptom**: Sandbox containers timeout or fail to connect to backend

**Diagnosis**:
```bash
curl http://localhost:8080/health
# Should return: ok
```

**Solution**:
1. Check backend service has `extra_hosts: - "host.docker.internal:host-gateway"`
2. Verify backend is accessible: `curl http://localhost:8080/health`

### Frontend Not Loading

**Symptom**: Frontend shows blank page or connection errors

**Diagnosis**:
```bash
curl http://localhost:3000
curl http://localhost:3000/api/v1/health  # Should proxy to backend
```

**Solution**:
1. Check frontend container is running (`docker compose ps`)
2. Check `nginx.conf` is mounted correctly
3. Check backend is healthy (frontend proxies `/api/` to backend)
4. Clear browser cache and reload

### Database Connection Errors

**Symptom**: "Connection refused" or "could not connect to server" errors

**Diagnosis**:
```bash
docker compose -f docker-compose/docker-compose.yml ps postgres
docker compose -f docker-compose/docker-compose.yml logs postgres
```

**Solution**:
1. Check PostgreSQL is running and healthy
2. Verify `DATABASE_URL` matches PostgreSQL service configuration
3. Check credentials match `POSTGRES_USER` and `POSTGRES_PASSWORD`
4. Wait for database initialization (can take 10-30 seconds on first start)

### Port Already in Use

**Symptom**: "bind: address already in use" error

**Solution**: Change ports in `.env`:
```bash
FRONTEND_PORT=3001
BACKEND_PORT=8081
```

### Health Check Failing

Run comprehensive health check:
```bash
docker-compose/docker-compose-verify.sh
```

This checks PostgreSQL, backend API, frontend, Docker socket access, and database schema.

## Production Deployment

### Security Considerations

1. **Docker Socket Access** — Consider using [Docker Socket Proxy](https://github.com/Tecnativa/docker-socket-proxy)
2. **Environment Variables** — Use Docker secrets or external secret management; never commit `.env`; rotate `TASK_TOKEN_SECRET` regularly
3. **Network Isolation** — Restrict database access to backend only
4. **Resource Limits** — Add memory and CPU limits to prevent resource exhaustion
5. **TLS/SSL** — Use reverse proxy for HTTPS with Let's Encrypt certificates

### Production Checklist

- [ ] Change all default passwords
- [ ] Generate secure random key for `TASK_TOKEN_SECRET`
- [ ] Configure SSL/TLS certificates
- [ ] Set resource limits for all services
- [ ] Configure log rotation
- [ ] Set up monitoring
- [ ] Configure backup automation
- [ ] Test backup and restore procedures

## Additional Resources

- [Main README](../README.md)
- [Docker Compose Documentation](https://docs.docker.com/compose/)
