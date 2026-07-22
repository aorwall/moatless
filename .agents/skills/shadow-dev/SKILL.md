---
name: shadow-dev
description: "Run the moatless-vibe backend + frontend from INSIDE a moatless sandbox and preview the full app. The backend runs as a managed server on :8080 (scripts/shadow/up.sh), the frontend dev server proxies /api to it, all against shared dev Postgres/Redis/S3. Use when asked to run the shadow backend, develop moatless-vibe from inside a sandbox, or preview/rebuild backend changes live (scripts/shadow)."
---

# Shadow Dev — run moatless-vibe from inside a sandbox

Run the moatless-vibe **backend + frontend as sidecar servers in a sandbox pod**
and preview the whole app at the sandbox's preview URLs — no laptop, no redeploy.
The backend runs on `:8080` via `scripts/shadow/up.sh`; the frontend dev server
proxies `/api` to it. Both talk to the shared **dev** Postgres/Redis/S3.

Use it when iterating on `backend/src/**`, `crates/**`, or `apps/frontend/src/**`
and you want a real, cluster-connected instance instead of Docker Compose or a
full K8s redeploy.

## 1. Set up a dedicated shadow repo (one time)

Run the shadow backend as a **server** on its own **repository record** — never
add it to the shared/global `moatless-vibe` repo (that repo is global, so every
task, agents included, would then compile Rust on start and run under the
elevated `moatless-dev` SA).

```bash
moat repos create \
  --name "moatless-vibe (shadow)" \
  --git-url https://github.com/soaplabs/moatless-vibe --branch main \
  --sandbox-service-account moatless-dev \
  --docker-access true --enable-inspector true \
  --storage 40Gi --storage-mode s3 \
  --setup-command 'curl -fsSL https://bun.sh/install | BUN_INSTALL=/opt/moatless/data/.bun bash' \
  --setup-command '/opt/moatless/data/.bun/bin/bun install' \
  --servers-json '[
    {"name":"dev","label":"Dev Server","port":3000,"default":true,
     "startCommands":["/opt/moatless/data/.bun/bin/bun run dev -- --host 0.0.0.0"],
     "startupTimeoutSeconds":120,"autoReload":true,"enableInspector":true},
    {"name":"backend","label":"Shadow Backend","port":8080,
     "startCommands":["bash scripts/shadow/up.sh"],"startupTimeoutSeconds":600}
  ]'
```

`--sandbox-service-account moatless-dev` lets the shadow backend provision
sandboxes; it needs the Helm `shadowDev.enabled=true` RBAC deployed (see
`scripts/shadow/README.md`). Drop the flag if you don't need provisioning.

## 2. Run

Create a task on the **`moatless-vibe (shadow)`** repo. Provisioning builds a
`backend-server` container (runs `up.sh`) plus the `dev` frontend. `up.sh`:
syncs the live backend env → `cargo build --release` → runs the binary on `:8080`,
restarting on rebuilds.

Open the **`--3000`** preview URL for the full app (frontend proxies `/api` to
the sibling backend); **`--8080`** hits the backend directly.

Verify (from any container in the pod):

```bash
curl -s -o /dev/null -w "backend :8080/health -> %{http_code}\n" http://localhost:8080/health
curl -s -o /dev/null -w "proxy   :3000/api    -> %{http_code}\n" http://localhost:3000/api/v1/repositories
moat tasks logs $TASK_ID --server backend      # follow the build/run logs
```

Healthy backend logs `API server started port=8080`.

## 3. Rebuild loop

```bash
bash scripts/shadow/rebuild.sh        # compile + signal up.sh to restart the binary
```

- **Frontend edits** hot-reload — no rebuild.
- **Env changed** on the real Deployment? `bash scripts/shadow/sync-env.sh` then `rebuild.sh`.
- **Extra overrides**: add `KEY=value` lines to `/opt/moatless/data/shadow/overrides.env` (e.g. `RUST_LOG=debug`).

Before considering a change done:
`cargo clippy -p moatless-vibe-backend -- -D warnings && cargo test -p moatless-vibe-backend`.

## Fast rebuilds (precompiled image)

The plain sandbox image recompiles all deps on every provision (~6–10 min).
`docker/Dockerfile.sandbox-dev` bakes warm caches — the cargo target + crate
registry (so only changed workspace crates recompile, ~1–2 min) and bun's global
install cache (so provision-time `bun install` links node_modules offline and the
frontend dev server starts fast). CI publishes it as `moatless-dev:latest` via an
**on-demand** job — the `Build dev sandbox image` workflow
(`.github/workflows/build-dev-image.yml`), run manually from the Actions tab (not
part of push-to-main). Normally just point the shadow repo at it:

```bash
moat repos update <shadow_repo_id> --docker-image <ecr-registry>/moatless-dev:latest
```

To build it by hand:

```bash
docker build -f docker/Dockerfile.sandbox-dev \
  --build-arg SANDBOX_IMAGE=aorwall/moatless-sandbox:<tag> \
  -t aorwall/moatless-dev:<tag> .   # + docker push
```

## Isolation (shared dev state — important)

The shadow reads/writes the **shared dev Postgres/Redis/S3** — treat it as live
dev data. `sync-env.sh` fences off shared-state consumers so the shadow can't
disrupt the real backend: distinct `REDIS_CONSUMER_GROUP`/`ID`,
`EVENT_HANDLER_ENABLED=false`, `SANDBOX_IDLE_REAPER_ENABLED=false` +
`RECONCILE_*_ENABLED=false` (so it never stops/reprovisions real sandboxes), and
`RUN_MIGRATIONS=false` (so it never mutates the shared dev schema — point
`DATABASE_URL` at your own DB in `overrides.env` + `RUN_MIGRATIONS=true` if you
need new schema).

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| `backend-server` never ready | `moat tasks logs $TASK_ID --server backend` — usually still compiling (ensure `startupTimeoutSeconds: 600`) or panicked on boot. |
| `/health` never 200 | Check the logs for a build error or a bad synced env var. |
| Build errors | Run `cargo build --release -p moatless-vibe-backend` directly for full output. |
| Proxy `/api/...` 404 | Use a real path (`/api/v1/repositories`); confirm `:8080` is up first. |
| `sync-env.sh` fails | Needs `get deployments`/`get secrets` in `moatless`: `kubectl auth can-i get secrets -n moatless`. |
| Can't provision sandboxes | Needs the Helm `shadowDev` RBAC + `--sandbox-service-account moatless-dev`. |
| Put backend server on the global repo by mistake | `moat repos sync-config <repo_id>` restores `servers`; clear the SA directly (API can't null it): `UPDATE repositories SET sandbox_service_account=NULL WHERE id='<repo_id>';`. |

## Files

- `scripts/shadow/up.sh` — sync env → build → run backend, watch for rebuilds
- `scripts/shadow/rebuild.sh` — compile + trigger a restart
- `scripts/shadow/sync-env.sh` — live backend env + secrets → sourceable env file
- `scripts/shadow/config.shadow.json` — the servers config as a committed `.moatless/config.json` (branch alternative to `moat repos create`)
- `docker/Dockerfile.sandbox-dev` — precompiled image (published on demand as `moatless-dev:latest` by `.github/workflows/build-dev-image.yml`) with warm caches
- `scripts/shadow/README.md` — deeper details (RBAC, owning child sandboxes)
