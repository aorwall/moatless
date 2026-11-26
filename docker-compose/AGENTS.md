# Docker Compose deployment

## Starting everything
```bash
docker compose -f docker-compose/docker-compose.yml up -d
```
- Backend: http://localhost:8080

The stack serves no UI: it has no T3 Code service; see
`docs/deferred-changes.md`.

## Building the backend locally
```bash
docker-compose/dev-docker.sh   # or `bun run dev:docker`; builds the backend
                               # image, then starts every service
```
`docker/AGENTS.md` covers the other image builds.

## Checking, reading and stopping the stack
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

## Configuring the environment

`docker-compose.yml` names every service, image and port, and every port is an
overridable variable rather than the fixed number the ports above default to.

**Two install paths read two different `.env` files.** The scripts here take the
project root's, seeded from `./.env.example` — `docker-compose-setup.sh` passes
`--env-file ../.env`. `moat docker` takes `<compose-dir>/.env`, seeded from
`docker-compose/.env.example`; the `docker-install` skill owns that path. Edit
the one your install path reads.

`TASK_TOKEN_SECRET` is the only variable the stack refuses to start without.
Everything else carries a default, so a missing value surfaces as a broken
feature and not as a startup error.

`DATA_DIR` must be an absolute **host** path. The backend creates sibling
containers and hands them this path directly. A path that means something only
inside the backend container gives those siblings nothing.
