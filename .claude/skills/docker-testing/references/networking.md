# Docker Networking

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────┐
│ Docker Host                                                  │
│                                                              │
│  ┌──────────────────┐     ┌──────────────────────────────┐  │
│  │ moatless-backend │     │ moatless-moatable-{taskId}   │  │
│  │                  │     │                              │  │
│  │ Port 8000        │────▶│ Port 8001 (mapped to host)   │  │
│  │                  │     │                              │  │
│  │ Uses:            │     │ ┌────────────────────────┐   │  │
│  │ SANDBOX_HOST     │     │ │ Dev Server (Vite/etc)  │   │  │
│  │ = host.docker    │     │ │ Port 3000 (internal)   │   │  │
│  │   .internal      │     │ └────────────────────────┘   │  │
│  └──────────────────┘     └──────────────────────────────┘  │
│                                                              │
│  ┌──────────────────┐     ┌──────────────────────────────┐  │
│  │ moatless-redis   │     │ moatless-postgres            │  │
│  │ Port 6379        │     │ Port 5432                    │  │
│  └──────────────────┘     └──────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

## SANDBOX_HOST Environment Variable

Backend needs to reach dynamically created sandbox containers. The approach differs based on where backend runs:

| Backend Location | SANDBOX_HOST Value | Why |
|------------------|-------------------|-----|
| Host machine | `localhost` (default) | Port mappings accessible on localhost |
| Docker container | `host.docker.internal` | Must route through Docker host |

### Configuration

**docker-compose.yml**:
```yaml
backend:
  environment:
    SANDBOX_HOST: host.docker.internal
```

**Code** (`docker-workspace-provisioner.ts`):
```typescript
const SANDBOX_HOST = process.env['SANDBOX_HOST'] || 'localhost'
// Used for health checks, agent status, worker URLs
```

## Container Communication Patterns

### Backend → Sandbox
```
Backend container
    │
    ▼ http://${SANDBOX_HOST}:${hostPort}/health
    │
Docker host (host.docker.internal)
    │
    ▼ Port mapping (e.g., 58164:8001)
    │
Sandbox container port 8001
```

### Backend → Redis/Postgres
Uses Docker network directly:
```yaml
backend:
  depends_on:
    - redis
    - postgres
  environment:
    REDIS_HOST: redis
    DB_HOST: postgres
```

## Port Mappings

### Static (docker-compose)
| Service | Container Port | Host Port |
|---------|---------------|-----------|
| Backend | 8000 | 8000 |
| Redis | 6379 | 6379 |
| Postgres | 5432 | 5432 |

### Dynamic (sandbox containers)
Sandbox containers get random host ports:
```bash
# Find sandbox port
docker port moatless-moatable-abc12345 8001
# Output: 0.0.0.0:58164
```

## host.docker.internal

Special DNS name that resolves to the Docker host from inside containers.

**Docker Desktop (Mac/Windows)**: Works automatically

**Linux**: Requires explicit mapping:
```yaml
services:
  backend:
    extra_hosts:
      - "host.docker.internal:host-gateway"
```

Already configured in `docker-compose.yml`.

## Troubleshooting

### Backend Can't Reach Sandbox
**Symptom**: Timeout when starting task

**Check**:
```bash
# Verify SANDBOX_HOST is set
docker exec moatless-backend printenv SANDBOX_HOST
# Should output: host.docker.internal

# Check sandbox container exists and has port mapping
docker ps --format "table {{.Names}}\t{{.Ports}}" | grep moatable
```

### Sandbox Container Not Accessible
```bash
# Get the mapped port
PORT=$(docker port moatless-moatable-{taskId} 8001 | cut -d: -f2)

# Test from host
curl http://localhost:$PORT/health

# Test from backend container (simulates what backend does)
docker exec moatless-backend curl http://host.docker.internal:$PORT/health
```
