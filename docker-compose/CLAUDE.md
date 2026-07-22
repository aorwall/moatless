# Docker Compose Deployment

## Start Everything
```bash
docker compose -f docker-compose/docker-compose.yml up -d
```
- Frontend (nginx + SPA): http://localhost:3000
- Backend API proxied via frontend at /api/
- Backend direct: http://localhost:8080

## Frontend Dev (hot-reload)
Also run `bun run dev` and use http://localhost:5173 instead.
Vite proxies `/api/` to the backend automatically.

## Dev Docker (builds backend locally)
```bash
docker-compose/dev-docker.sh   # builds backend image, starts all services
bun run dev                     # optional: frontend with hot-reload
```

## Key Services
- **frontend**: `aorwall/moatless-frontend` (nginx serving React SPA + API proxy)
- **backend**: `aorwall/moatless-vibe-backend` (Rust/Axum on port 8080)
- **postgres**: PostgreSQL 17 on port 5432
- **redis**: Redis 7 on port 6379
- **dozzle**: Log viewer on port 9999

## Building Images Locally
```bash
docker/build-app.sh --local       # frontend nginx image
docker/build-sandbox.sh --local   # sandbox images
# Backend image is built by dev-docker.sh automatically
```

## Useful Commands
```bash
# Health check
docker-compose/docker-compose-verify.sh

# View logs
docker-compose/logs.sh

# E2E test
python docker-compose/e2e-test.py

# Stop everything
docker compose -f docker-compose/docker-compose.yml down
```

## Environment
- Config: `.env` (copy from `.env.example` in project root)
- Required: `CLAUDE_CODE_OAUTH_TOKEN`, `TASK_TOKEN_SECRET`, `DATA_DIR`
- Sandbox provider: `docker` (set automatically in compose)
