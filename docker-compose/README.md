# Docker Compose Deployment Guide

This guide covers deploying Moatless Vibe using Docker Compose with PostgreSQL, Redis, and Docker-based sandbox provisioning.

## Table of Contents

1. [Quick Start](#quick-start)
2. [Architecture](#architecture)
3. [Configuration](#configuration)
4. [Common Operations](#common-operations)
5. [Troubleshooting](#troubleshooting)
6. [Production Deployment](#production-deployment)
7. [Development Mode](#development-mode)

## Quick Start

### Prerequisites

- Docker 20.10+ and Docker Compose v2+ installed
- At least 4GB RAM available
- Port 3000, 5432, 6379, and 8000 available

### Setup

1. **Clone the repository**
   ```bash
   git clone https://github.com/aorwall/moatless-vibe.git
   cd moatless-vibe
   ```

2. **Run setup script**
   ```bash
   chmod +x docker-compose-setup.sh
   ./docker-compose-setup.sh
   ```

   The script will:
   - Check Docker prerequisites
   - Create `.env.docker-compose` from template
   - Prompt you to configure required values
   - Pull Docker images
   - Start all services

3. **Configure required values in `.env.docker-compose`**
   - `CLAUDE_CODE_OAUTH_TOKEN` - Get from https://claude.ai/settings/code
   - `WORKER_API_KEY` - Generate with: `node -e "console.log(require('crypto').randomBytes(32).toString('hex'))"`
   - `SESSION_SECRET` - Generate with same command
   - `POSTGRES_PASSWORD` - Change from default

4. **Verify deployment**
   ```bash
   ./docker-compose-verify.sh
   ```

5. **Access the application**
   - Frontend: http://localhost:3000
   - Backend API: http://localhost:8000
   - API Health: http://localhost:8000/health

## Architecture

### System Overview

```
┌─────────────┐     ┌─────────────┐
│  Frontend   │────▶│   Backend   │
│   :3000     │     │    :8000    │
└─────────────┘     └──────┬──────┘
                           │
                    ┌──────┴──────┬──────────────┐
                    ▼             ▼              ▼
              ┌──────────┐  ┌─────────┐   ┌──────────────┐
              │PostgreSQL│  │  Redis  │   │Docker Socket │
              │  :5432   │  │  :6379  │   │ (sibling     │
              └──────────┘  └─────────┘   │  containers) │
                                          └──────────────┘
                                                 │
                                          ┌──────▼──────┐
                                          │  Sandbox    │
                                          │ Containers  │
                                          └─────────────┘
```

### Services

#### PostgreSQL
- **Image**: postgres:17-alpine
- **Purpose**: Primary database for tasks, iterations, messages, events, repositories
- **Port**: 5432 (configurable)
- **Data**: Persisted in `postgres_data` volume

#### Redis
- **Image**: redis:7-alpine
- **Purpose**: Event streaming transport using Redis Streams
- **Port**: 6379 (configurable)
- **Configuration**: 512MB memory limit with LRU eviction, AOF persistence
- **Data**: Persisted in `redis_data` volume

#### Backend
- **Image**: ghcr.io/aorwall/moatless-vibe:latest
- **Purpose**: Main application server, API gateway, sandbox orchestration
- **Port**: 8000 (configurable)
- **Key Features**:
  - Docker socket access for sandbox provisioning
  - Automatic database migrations
  - Event streaming via Redis
  - MCP server for agent tools

#### Frontend
- **Image**: node:22-alpine (development mode)
- **Purpose**: React application with Vite dev server
- **Port**: 3000 (configurable)
- **Note**: In production, serve frontend via backend or CDN

### Network Flow

1. **Frontend → Backend**: HTTP API calls to `localhost:8000`
2. **Backend → PostgreSQL**: Database queries via service name `postgres:5432`
3. **Backend → Redis**: Event streaming via service name `redis:6379`
4. **Backend → Docker Socket**: Creates sibling sandbox containers via `/var/run/docker.sock`
5. **Sandbox → Backend**: HTTP API calls via `host.docker.internal:8000`

### Volume Mounts

| Volume | Container Path | Purpose |
|--------|---------------|---------|
| `postgres_data` | `/var/lib/postgresql/data` | PostgreSQL database files |
| `redis_data` | `/data` | Redis persistence (AOF) |
| `data_dir` | `/data` | Shared data for backend and sandboxes |
| Docker socket | `/var/run/docker.sock` | Container management (read-only) |

### Sandbox Workspace Provisioning

#### Architecture Overview

Each task gets its own isolated Docker sandbox container with the following architecture:

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

#### Per-Task Isolated Storage

**Key Design Principle**: Each task gets its own isolated storage directory, similar to Kubernetes Persistent Volume Claims (PVCs).

**Directory Structure**:
```
./data/
├── tasks/
│   ├── {task-id-1}/     # Per-task isolated storage
│   │   ├── logs/
│   │   ├── cache/
│   │   └── artifacts/
│   ├── {task-id-2}/
│   └── {task-id-3}/
├── repositories/         # Repository configurations
└── moatless.db          # SQLite database (development)
```

**Volume Mounts per Sandbox**:
- **Host**: `./data/tasks/{task-id}/` (unique per task)
- **Container**: `/data` (mounted read-write)
- **Purpose**: Persistent storage for logs, cache, build artifacts

**Workspace Directory**:
- **Container**: `/opt/moatless/repo` (ephemeral, lives in container)
- **Purpose**: Git repository workspace
- **Lifecycle**: Created on container start, destroyed on container removal

#### Git Clone Workflow

When a new task is created with a remote repository:

1. **Container Creation**
   - Backend calls Docker API to create sandbox container
   - Mounts per-task data directory: `./data/tasks/{task-id}/ → /data`
   - Sets environment variables:
     - `GIT_CLONE_URL`: Repository URL to clone
     - `WORKSPACE_DIR=/opt/moatless/repo`
     - `SETUP_COMMANDS`: Commands to run after clone (e.g., `npm install`)

2. **Container Startup**
   - Entrypoint script (`docker-entrypoint-sandbox.sh`) runs as root
   - Creates workspace directory with proper ownership:
     ```bash
     mkdir -p $WORKSPACE_DIR
     chown -R moatless:moatless $WORKSPACE_DIR
     ```
   - **Critical**: Workspace directory is NOT pre-created in Docker image
   - **Critical**: Setup commands run AFTER git clone completes

3. **Git Clone Execution**
   - Sandbox application starts as `moatless` user (not root)
   - Detects empty workspace directory
   - Executes: `git clone $GIT_CLONE_URL /opt/moatless/repo`
   - Clone succeeds because directory is empty and owned by moatless user

4. **Setup Commands**
   - After successful clone, runs setup commands (e.g., `npm install`, `pip install -r requirements.txt`)
   - Creates build artifacts, node_modules, etc.
   - All files owned by moatless user

5. **Workspace Ready**
   - Sandbox reports "ready" status to backend via HTTP API
   - Claude Code agent can now execute commands in workspace

#### Container Restart Behavior

**Existing Task (Container Restart)**:
- Workspace directory exists from previous run
- Detects `.git` directory present
- Skips git clone (uses existing workspace)
- Runs setup commands only if needed
- Preserves all files from previous session

**New Container (First Start)**:
- Workspace directory does not exist
- Git clone succeeds to empty directory
- Setup commands run after clone
- Fresh workspace with no conflicts

#### Common Workspace Issues

##### Issue: "directory already exists and is not an empty directory"

**Root Cause**: Workspace directory pre-created in Docker image or by incorrect `-w` flag usage.

**Fixed By**:
1. **Dockerfile.sandbox**: Removed `/opt/moatless/repo` from directory creation
   ```dockerfile
   # Before (WRONG):
   RUN mkdir -p /opt/moatless/repo /opt/moatless/data

   # After (CORRECT):
   RUN mkdir -p /opt/moatless/data /opt/moatless/.claude
   # Note: Do NOT create /opt/moatless/repo - must be empty for git clone
   ```

2. **docker.service.ts**: Removed `-w` flag from docker create command
   ```typescript
   // Before (WRONG):
   docker create -w "/opt/moatless/repo" ...

   // After (CORRECT):
   docker create ...  # No -w flag - creates directory as root
   ```

3. **docker-entrypoint-sandbox.sh**: Moved setup commands after git clone
   ```bash
   # Setup commands NOW run by sandbox app AFTER git clone
   # Not by entrypoint BEFORE git clone
   ```

##### Issue: "Permission denied" creating .git directory

**Root Cause**: Workspace directory owned by root, moatless user can't write.

**Fixed By**:
1. Entrypoint creates workspace with correct ownership:
   ```bash
   mkdir -p "$WORKSPACE_DIR"
   chown -R moatless:moatless "$WORKSPACE_DIR"
   ```

2. Removed `-w` flag that was causing Docker to create directory as root

##### Issue: Workspace contains files from different repository

**Root Cause**: Container reused for different task without cleanup.

**Prevention**:
- Each task gets unique container name: `moatless-{repo}-{task-id}`
- Per-task data directory ensures isolation
- Container removal cleans up workspace

#### Verification Commands

Check sandbox container status:
```bash
# List all sandbox containers
docker ps -a --filter "label=created-by=moatless"

# Check specific task's container
docker ps -a --filter "label=moatless.task-id={task-id}"

# View container logs
docker logs moatless-{repo}-{task-id}

# Check git clone status
docker logs moatless-{repo}-{task-id} 2>&1 | grep -E "Cloning|cloned successfully"
```

Check workspace directory:
```bash
# List workspace files
docker exec moatless-{repo}-{task-id} ls -la /opt/moatless/repo

# Check ownership
docker exec moatless-{repo}-{task-id} ls -ld /opt/moatless/repo
# Expected: drwxr-xr-x ... moatless moatless ... /opt/moatless/repo

# Check running user
docker exec moatless-{repo}-{task-id} whoami
# Expected: moatless
```

Check per-task data directory:
```bash
# List task directories
ls -la ./data/tasks/

# Check specific task's data
ls -la ./data/tasks/{task-id}/

# Verify mount in container
docker inspect moatless-{repo}-{task-id} | jq '.[0].Mounts | map(select(.Destination == "/data"))'
```

#### Environment Variables for Workspace

| Variable | Purpose | Example |
|----------|---------|---------|
| `WORKSPACE_DIR` | Workspace directory path | `/opt/moatless/repo` |
| `DATA_DIR` | Per-task data directory | `/data` |
| `GIT_CLONE_URL` | Repository URL to clone | `https://github.com/user/repo.git` |
| `SETUP_COMMANDS` | JSON array of commands | `["npm install", "bun run build"]` |
| `WORKER_MODE` | Enable worker mode | `true` |
| `WORKER_API_URL` | Backend API URL | `http://host.docker.internal:8000` |
| `WORKER_API_KEY` | Authentication key | Generated random key |

#### Workspace Execution Mode

Set workspace provisioning mode via environment variable:

**Docker Mode** (uses sibling containers):
```bash
WORKSPACE_EXECUTION_MODE=docker
```

**Kubernetes Mode** (uses Kubernetes pods):
```bash
WORKSPACE_EXECUTION_MODE=kubernetes
```

Default is Kubernetes if not specified. For Docker Compose deployments, always set to `docker`.

## Configuration

### Required Environment Variables

Edit `.env.docker-compose` and set these required values:

#### 1. Claude Authentication (REQUIRED)
```bash
CLAUDE_CODE_OAUTH_TOKEN=your_token_here
```
Get from: https://claude.ai/settings/code

Alternatively, use Anthropic API key:
```bash
ANTHROPIC_API_KEY=sk-ant-...
```

#### 2. Worker API Security (REQUIRED)
```bash
WORKER_API_KEY=$(node -e "console.log(require('crypto').randomBytes(32).toString('hex'))")
```
This secures communication between sandboxes and the backend.

#### 3. Session Security (REQUIRED if using auth)
```bash
SESSION_SECRET=$(node -e "console.log(require('crypto').randomBytes(32).toString('hex'))")
```

#### 4. Database Password
```bash
POSTGRES_PASSWORD=your_secure_password_here
```
Change from default for security.

#### 5. Workspace Execution Mode (REQUIRED for Docker Compose)
```bash
WORKSPACE_EXECUTION_MODE=docker
```
**Critical**: Set to `docker` for Docker Compose deployments. This enables Docker-based sandbox provisioning instead of the default Kubernetes mode.

Without this setting, the backend will try to use Kubernetes and fail to provision sandboxes.

### Optional Configuration

#### Authentication Provider

Disable authentication (development):
```bash
AUTH_PROVIDER=
```

Or enable GitHub OAuth:
```bash
AUTH_PROVIDER=github
GITHUB_CLIENT_ID=your_client_id
GITHUB_CLIENT_SECRET=your_client_secret
GITHUB_CALLBACK_URL=http://localhost:8000/api/v1/auth/github/callback
```

Create OAuth app at: https://github.com/settings/developers

Or enable Auth0:
```bash
AUTH_PROVIDER=auth0
AUTH0_DOMAIN=your-tenant.auth0.com
AUTH0_CLIENT_ID=your_client_id
AUTH0_CLIENT_SECRET=your_client_secret
AUTH0_CALLBACK_URL=http://localhost:8000/api/v1/auth/auth0/callback
```

#### Sandbox Image

Choose sandbox environment:

**JavaScript/TypeScript only** (default, 2GB):
```bash
SANDBOX_IMAGE=ghcr.io/aorwall/moatless-vibe-sandbox-js:latest
```

**Full stack** (includes Python, Rust, Go, Java, 5-8GB):
```bash
SANDBOX_IMAGE=ghcr.io/aorwall/moatless-vibe-sandbox:latest
```

#### Port Configuration

Customize ports to avoid conflicts:
```bash
BACKEND_PORT=8000
FRONTEND_PORT=3000
POSTGRES_PORT=5432
REDIS_PORT=6379
```

#### Claude Config Directory

Mount local Claude configuration (optional):
```bash
CLAUDE_CONFIG_DIR=~/.claude
```

This allows sandboxes to use your local Claude OAuth tokens.

### Environment Variable Reference

See `.env.docker-compose.example` for complete list with descriptions.

## Common Operations

### Start Services

```bash
docker-compose --env-file .env.docker-compose up -d
```

### View Logs

All services:
```bash
docker-compose --env-file .env.docker-compose logs -f
```

Specific service:
```bash
docker-compose --env-file .env.docker-compose logs -f backend
docker-compose --env-file .env.docker-compose logs -f postgres
docker-compose --env-file .env.docker-compose logs -f redis
```

### Stop Services

```bash
docker-compose --env-file .env.docker-compose down
```

### Restart Services

```bash
docker-compose --env-file .env.docker-compose restart
```

Restart specific service:
```bash
docker-compose --env-file .env.docker-compose restart backend
```

### Update to Latest Version

```bash
docker-compose --env-file .env.docker-compose pull
docker-compose --env-file .env.docker-compose up -d
```

### View Service Status

```bash
docker-compose --env-file .env.docker-compose ps
```

### Execute Commands in Containers

PostgreSQL:
```bash
docker-compose --env-file .env.docker-compose exec postgres psql -U moatless -d moatless
```

Redis:
```bash
docker-compose --env-file .env.docker-compose exec redis redis-cli
```

Backend:
```bash
docker-compose --env-file .env.docker-compose exec backend sh
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
docker-compose --env-file .env.docker-compose down -v
docker-compose --env-file .env.docker-compose up -d
```

### Backup Database

```bash
docker-compose --env-file .env.docker-compose exec postgres pg_dump -U moatless moatless > backup.sql
```

Restore:
```bash
cat backup.sql | docker-compose --env-file .env.docker-compose exec -T postgres psql -U moatless -d moatless
```

## Troubleshooting

### Backend Can't Create Containers

**Symptom**: "Cannot connect to Docker daemon" error in backend logs

**Diagnosis**:
```bash
docker-compose --env-file .env.docker-compose exec backend docker ps
```

**Solution**:
1. Ensure Docker socket is mounted in `docker-compose.yml`
2. Check backend service has volume: `/var/run/docker.sock:/var/run/docker.sock:ro`
3. Restart backend: `docker-compose --env-file .env.docker-compose restart backend`

### Sandboxes Can't Reach Backend

**Symptom**: Worker containers timeout or fail to connect to backend

**Diagnosis**:
```bash
docker-compose --env-file .env.docker-compose exec backend env | grep WORKER_API_URL
# Should show: http://host.docker.internal:8000
```

**Solution**:
1. Verify `WORKER_API_URL` uses `host.docker.internal`
2. Check backend service has `extra_hosts: - "host.docker.internal:host-gateway"`
3. Verify backend is accessible: `curl http://localhost:8000/health`

### Workspace Provisioning Issues

#### Git Clone Fails: "directory already exists"

**Symptom**: Sandbox logs show `fatal: destination path '/opt/moatless/repo' already exists and is not an empty directory`

**Root Cause**: Workspace directory was pre-created in Docker image or container.

**Diagnosis**:
```bash
# Check if directory exists in image
docker run --rm ghcr.io/aorwall/moatless-vibe-sandbox-js:latest ls -la /opt/moatless/
# Should NOT show 'repo' directory

# Check container logs
docker logs moatless-{repo}-{task-id} 2>&1 | grep "already exists"
```

**Solution**:
1. Verify you're using the latest sandbox image with the fix:
   ```bash
   docker pull ghcr.io/aorwall/moatless-vibe-sandbox-js:latest
   # Or for full stack:
   docker pull ghcr.io/aorwall/moatless-vibe-sandbox:latest
   ```

2. Remove old containers and restart backend:
   ```bash
   docker rm -f $(docker ps -aq --filter "label=created-by=moatless")
   docker-compose --env-file .env.docker-compose restart backend
   ```

3. If using custom sandbox image, ensure `Dockerfile.sandbox` does NOT pre-create workspace:
   ```dockerfile
   # WRONG - Do not do this:
   RUN mkdir -p /opt/moatless/repo

   # CORRECT - Only create data and config directories:
   RUN mkdir -p /opt/moatless/data /opt/moatless/.claude
   ```

#### Git Clone Fails: "Permission denied"

**Symptom**: Sandbox logs show `Permission denied` when creating `.git` directory

**Root Cause**: Workspace directory owned by root, moatless user can't write.

**Diagnosis**:
```bash
# Check directory ownership
docker exec moatless-{repo}-{task-id} ls -ld /opt/moatless/repo
# Should show: drwxr-xr-x ... moatless moatless

# Check running user
docker exec moatless-{repo}-{task-id} whoami
# Should show: moatless
```

**Solution**:
1. Verify entrypoint creates directory with correct ownership
2. Ensure backend is NOT using `-w` flag in docker create command
3. Update to latest images with ownership fixes

#### Workspace Contains Wrong Repository

**Symptom**: Sandbox contains files from a different repository than expected

**Root Cause**: Container was reused or workspace wasn't properly cleaned.

**Solution**:
```bash
# Remove all sandbox containers
docker rm -f $(docker ps -aq --filter "label=created-by=moatless")

# Remove per-task data directories if needed
rm -rf ./data/tasks/

# Restart backend
docker-compose --env-file .env.docker-compose restart backend
```

#### Setup Commands Fail

**Symptom**: `npm install`, `pip install`, or other setup commands fail with file not found errors

**Root Cause**: Setup commands running before git clone completes.

**Diagnosis**:
```bash
# Check sandbox logs for command execution order
docker logs moatless-{repo}-{task-id} 2>&1 | grep -E "Cloning|setup command"
```

**Expected Order**:
1. "Cloning repository"
2. "Repository cloned successfully"
3. "Running setup command: npm install"

**Solution**:
Ensure you're using the latest sandbox image where setup commands run AFTER git clone.

#### Sandbox Container Exits Immediately

**Symptom**: Container status shows "Exited (1)" shortly after creation

**Diagnosis**:
```bash
# Check container logs for errors
docker logs moatless-{repo}-{task-id}

# Check container exit code
docker inspect moatless-{repo}-{task-id} | jq '.[0].State.ExitCode'
```

**Common Causes**:
1. **Missing environment variables**: Check `GIT_CLONE_URL`, `WORKER_API_URL`, `WORKER_API_KEY`
2. **Network issues**: Can't reach backend or git repository
3. **Invalid git URL**: Check repository URL is accessible
4. **Database connection**: SQLite database file not writable

**Solution**:
```bash
# Check environment variables
docker inspect moatless-{repo}-{task-id} | jq '.[0].Config.Env'

# Verify backend is accessible from sandbox
docker run --rm --network host curlimages/curl:latest curl -v http://host.docker.internal:8000/health

# Check git URL is valid
git ls-remote https://github.com/user/repo.git
```

#### Per-Task Data Directory Issues

**Symptom**: Sandbox can't write to `/data` directory

**Diagnosis**:
```bash
# Check mount exists
docker inspect moatless-{repo}-{task-id} | jq '.[0].Mounts | map(select(.Destination == "/data"))'

# Check host directory permissions
ls -ld ./data/tasks/{task-id}/

# Test write access in container
docker exec moatless-{repo}-{task-id} touch /data/test.txt
```

**Solution**:
1. Ensure `DATA_DIR` environment variable is set correctly in backend
2. Check host directory is writable by Docker
3. Verify mount is present in container

### Database Connection Errors

**Symptom**: "Connection refused" or "could not connect to server" errors

**Diagnosis**:
```bash
docker-compose --env-file .env.docker-compose ps postgres
docker-compose --env-file .env.docker-compose logs postgres
```

**Solution**:
1. Check PostgreSQL is running and healthy
2. Verify `DATABASE_URL` matches PostgreSQL service configuration
3. Check credentials match `POSTGRES_USER` and `POSTGRES_PASSWORD`
4. Wait for database initialization (can take 10-30 seconds on first start)

### Database Migrations Not Running

**Symptom**: "relation does not exist" or schema errors

**Diagnosis**:
```bash
docker-compose --env-file .env.docker-compose logs backend | grep migration
```

**Solution**:
Backend runs migrations automatically on startup. If they failed:
1. Check database connection is working
2. Restart backend: `docker-compose --env-file .env.docker-compose restart backend`
3. Check for migration errors in logs

### Redis Connection Errors

**Symptom**: "Redis connection failed" or event streaming not working

**Diagnosis**:
```bash
docker-compose --env-file .env.docker-compose exec redis redis-cli ping
# Should return: PONG

docker-compose --env-file .env.docker-compose exec backend env | grep REDIS_URL
# Should show: redis://redis:6379
```

**Solution**:
1. Verify Redis is running and healthy
2. Check `REDIS_URL` points to `redis://redis:6379`
3. Restart backend to reconnect

### Frontend Not Loading

**Symptom**: Frontend shows blank page or connection errors

**Diagnosis**:
```bash
docker-compose --env-file .env.docker-compose logs frontend
curl http://localhost:3000
```

**Solution**:
1. Check frontend container is running
2. Verify `VITE_API_URL` points to correct backend URL
3. Check backend is accessible from frontend
4. Clear browser cache and reload

### Port Already in Use

**Symptom**: "bind: address already in use" error

**Solution**:
1. Change ports in `.env.docker-compose`:
   ```bash
   BACKEND_PORT=8001
   FRONTEND_PORT=3001
   ```
2. Restart services

### Out of Disk Space

**Symptom**: "no space left on device" errors

**Solution**:
```bash
# Clean up sandbox containers
docker rm -f $(docker ps -aq --filter "label=created-by=moatless")

# Clean up unused images and volumes
docker system prune -a --volumes

# Check disk usage
docker system df
```

### Permission Denied on Docker Socket

**Symptom**: Backend can't access Docker socket

**Solution** (Linux):
```bash
# Add user to docker group
sudo usermod -aG docker $USER

# Or run with sudo (not recommended)
sudo docker-compose --env-file .env.docker-compose up -d
```

### Health Check Failing

Run comprehensive health check:
```bash
./docker-compose-verify.sh
```

This checks:
- Service status
- PostgreSQL connection
- Redis connection
- Backend API health
- Frontend availability
- Docker socket access
- Database schema

## Production Deployment

### Security Considerations

1. **Docker Socket Access**
   - Consider using [Docker Socket Proxy](https://github.com/Tecnativa/docker-socket-proxy)
   - Limits backend to only necessary Docker operations
   - Example configuration in production section below

2. **Environment Variables**
   - Use Docker secrets or external secret management (AWS Secrets Manager, HashiCorp Vault)
   - Never commit `.env.docker-compose` to version control
   - Rotate `WORKER_API_KEY` and `SESSION_SECRET` regularly

3. **Network Isolation**
   - Use custom networks with firewall rules
   - Restrict database and Redis access to backend only
   - Consider using private networks in cloud environments

4. **Resource Limits**
   - Add memory and CPU limits to prevent resource exhaustion
   - Set appropriate limits for sandbox containers
   - Monitor resource usage

5. **TLS/SSL**
   - Use reverse proxy (nginx, traefik) for HTTPS
   - Obtain SSL certificates (Let's Encrypt)
   - Force HTTPS for all connections

### Production Configuration

Create `docker-compose.prod.yml`:

```yaml
version: '3.8'

services:
  backend:
    deploy:
      resources:
        limits:
          cpus: '2'
          memory: 4G
        reservations:
          cpus: '1'
          memory: 2G
      restart_policy:
        condition: on-failure
        delay: 5s
        max_attempts: 3
    logging:
      driver: "json-file"
      options:
        max-size: "10m"
        max-file: "3"

  postgres:
    deploy:
      resources:
        limits:
          cpus: '1'
          memory: 2G
        reservations:
          cpus: '0.5'
          memory: 1G
    command: postgres -c max_connections=100 -c shared_buffers=256MB

  redis:
    deploy:
      resources:
        limits:
          cpus: '0.5'
          memory: 512M
        reservations:
          cpus: '0.25'
          memory: 256M
```

Deploy with production overrides:
```bash
docker-compose -f docker-compose.yml -f docker-compose.prod.yml --env-file .env.docker-compose up -d
```

### External Databases

For production, consider using managed databases:

**PostgreSQL** (AWS RDS, Azure Database, Google Cloud SQL):
```bash
DATABASE_PROVIDER=postgres
DATABASE_URL=postgresql://user:password@rds-endpoint:5432/dbname
```

Remove postgres service from docker-compose.yml

**Redis** (AWS ElastiCache, Azure Cache, Google Memorystore):
```bash
REDIS_URL=redis://redis-endpoint:6379
```

Remove redis service from docker-compose.yml

### Reverse Proxy with nginx

Create `nginx/nginx.conf`:
```nginx
upstream backend {
    server backend:8000;
}

upstream frontend {
    server frontend:3000;
}

server {
    listen 80;
    server_name your-domain.com;

    location / {
        proxy_pass http://frontend;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection 'upgrade';
        proxy_set_header Host $host;
        proxy_cache_bypass $http_upgrade;
    }

    location /api/ {
        proxy_pass http://backend;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
    }
}
```

Add to docker-compose.yml:
```yaml
services:
  nginx:
    image: nginx:alpine
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - ./nginx/nginx.conf:/etc/nginx/nginx.conf:ro
      - ./nginx/ssl:/etc/nginx/ssl:ro
    depends_on:
      - backend
      - frontend
    networks:
      - moatless
```

### Monitoring

Add Prometheus and Grafana:

```yaml
services:
  prometheus:
    image: prom/prometheus
    volumes:
      - ./prometheus.yml:/etc/prometheus/prometheus.yml
      - prometheus_data:/prometheus
    ports:
      - "9090:9090"
    networks:
      - moatless

  grafana:
    image: grafana/grafana
    volumes:
      - grafana_data:/var/lib/grafana
    ports:
      - "3001:3000"
    environment:
      - GF_SECURITY_ADMIN_PASSWORD=admin
    networks:
      - moatless

volumes:
  prometheus_data:
  grafana_data:
```

### Backup Automation

Create backup script `backup.sh`:
```bash
#!/bin/bash
BACKUP_DIR=/backups
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

# Backup PostgreSQL
docker-compose exec -T postgres pg_dump -U moatless moatless > $BACKUP_DIR/postgres_$TIMESTAMP.sql

# Backup Redis
docker-compose exec -T redis redis-cli BGSAVE

# Compress backups
tar -czf $BACKUP_DIR/backup_$TIMESTAMP.tar.gz $BACKUP_DIR/*_$TIMESTAMP.sql

# Upload to S3 (optional)
aws s3 cp $BACKUP_DIR/backup_$TIMESTAMP.tar.gz s3://your-bucket/backups/

# Clean up old backups (keep last 7 days)
find $BACKUP_DIR -name "backup_*.tar.gz" -mtime +7 -delete
```

Add to crontab:
```bash
0 2 * * * /path/to/backup.sh
```

### Production Checklist

- [ ] Change all default passwords
- [ ] Generate secure random keys for `WORKER_API_KEY` and `SESSION_SECRET`
- [ ] Configure SSL/TLS certificates
- [ ] Set up external databases (RDS, ElastiCache)
- [ ] Configure backup automation
- [ ] Set resource limits for all services
- [ ] Configure log rotation
- [ ] Set up monitoring (Prometheus, Grafana, Datadog)
- [ ] Configure alerting for service failures
- [ ] Set up log aggregation (ELK, CloudWatch)
- [ ] Review and harden security (firewalls, Docker socket proxy)
- [ ] Document disaster recovery procedures
- [ ] Test backup and restore procedures

## Development Mode

For local development with hot-reload:

```bash
docker-compose -f docker-compose.yml -f docker-compose.dev.yml --env-file .env.docker-compose up
```

This uses `docker-compose.dev.yml` which:
- Builds images locally instead of pulling from registry
- Mounts source directories for hot-reload
- Sets `NODE_ENV=development` and `LOG_LEVEL=debug`

### Local Development Setup

1. **Install dependencies**:
   ```bash
   bun install
   ```

2. **Build locally**:
   ```bash
   bun run build
   ```

3. **Start with development overrides**:
   ```bash
   docker-compose -f docker-compose.yml -f docker-compose.dev.yml --env-file .env.docker-compose up
   ```

4. **Make changes**: Edit source files and they'll be reflected in containers

5. **Rebuild when needed**:
   ```bash
   bun run build
   docker-compose -f docker-compose.yml -f docker-compose.dev.yml --env-file .env.docker-compose restart backend
   ```

## Additional Resources

- [Architecture Overview](./architecture-overview.md)
- [Main README](../README.md)
- [Claude Code Documentation](https://docs.anthropic.com/code)
- [Docker Compose Documentation](https://docs.docker.com/compose/)

## Support

For issues or questions:
- GitHub Issues: https://github.com/aorwall/moatless-vibe/issues
- Documentation: https://github.com/aorwall/moatless-vibe/docs
