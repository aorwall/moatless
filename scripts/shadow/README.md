# Shadow backend — mirrord-style dev, without mirrord

Run the moatless-vibe **backend + frontend inside a moatless sandbox** and use it
like the real backend. mirrord exists to bridge a laptop *into* a cluster; a
sandbox is **already a pod in the dev cluster**, so the hard parts (outgoing
traffic, DNS, cluster identity) are free or native. This harness reproduces the
rest.

## What maps to what

| mirrord capability | Native equivalent here |
| --- | --- |
| Outgoing traffic + DNS (reach Postgres, `moatless-vibe-redis-replicas:6379`, K8s API, sandbox ClusterIPs) | **Free** — the sandbox is on the pod network already |
| Env / secret sync from the target | `sync-env.sh` reads the live `moatless-vibe-backend` Deployment env + resolves `moatless-secrets` |
| Copy the target's ServiceAccount token (RBAC to create pods) | Helm `shadowDev` toggle (`templates/shadow-dev-rbac.yaml`) binds a `moatless-dev` SA to the backend's existing Roles |
| Steal incoming traffic | Not needed for the frontend (vite proxies `/api` → `localhost:8080`); for sandbox gRPC callbacks the backend self-advertises its address (Phase 3) |

## How a preview server actually runs (mechanism)

A preview server is a **container in the sandbox pod**, generated from the repo's
`servers` list — not a hand-started process:

- `build_server_containers()` (`backend/src/sandbox/k8s/pod_builder.rs`) makes one
  `{name}-server` container per `ServerConfig`, running its `startCommands` after
  the workspace-ready sentinel.
- Each server port gets a Traefik IngressRoute `{task_id}--{port}.{proxy_domain}`
  (`k8s/ingress.rs`) — the `…--3000.preview…` URL. Backend on 8080 → `…--8080…`.
- Readiness = TCP probe on the port. All pod containers share the network
  namespace, so the frontend's vite `/api → localhost:8080` reaches a sibling
  backend container.
- `servers` come from the repo DB record, synced from **`.moatless/config.json`**
  (`repository/config_reader.rs`).

So the shadow backend should run **as a server** with
`startCommands: ["bash scripts/shadow/up.sh"]`. Put it on a **dedicated
repository record** — **not** the shared/global `moatless-vibe` repo. That repo
is global, so adding the `backend` server (or `sandboxServiceAccount`) there
makes **every** moatless-vibe task (agents included) compile Rust on start and
run under the elevated `moatless-dev` SA. Keep it isolated:

```bash
# User-scoped dev repo pointing at the same GitHub repo (only you use it):
moat repos create --name "moatless-vibe (shadow)" \
  --git-url https://github.com/soaplabs/moatless-vibe --branch main \
  --sandbox-service-account moatless-dev --docker-access true \
  --storage 40Gi --storage-mode s3 \
  --setup-command 'curl -fsSL https://bun.sh/install | BUN_INSTALL=/opt/moatless/data/.bun bash' \
  --setup-command '/opt/moatless/data/.bun/bin/bun install' \
  --servers-json '[
    {"name":"dev","port":3000,"default":true,"autoReload":true,"enableInspector":true,
     "startCommands":["/opt/moatless/data/.bun/bin/bun run dev -- --host 0.0.0.0"],"startupTimeoutSeconds":30},
    {"name":"backend","label":"Shadow Backend","port":8080,
     "startCommands":["bash scripts/shadow/up.sh"],"startupTimeoutSeconds":600}]'
```

`config.shadow.json` is the equivalent as a committed `.moatless/config.json` if
you'd rather wire it via a shadow branch. Provisioning a task on the shadow repo
builds a `backend-server` container (visible in the sandbox view,
`moat tasks logs $TASK_ID --server backend`) alongside the `dev` frontend. Open
the `--3000` preview URL; the frontend proxies `/api` to the sibling backend.

> Put the `backend` server on the global repo by mistake? Revert with
> `moat repos sync-config <repo_id>` (restores `servers` from `main`), and clear
> the SA directly — the update API can't null it —
> `UPDATE repositories SET sandbox_service_account=NULL WHERE id='<repo_id>';`.

## Phase 1 — shadow app (works with today's sandbox permissions)

No cluster changes. The sandbox SA can already `get deployments` and
`get secrets`, and `cargo` + `bun` are in the sandbox image.

- **Preferred (managed server):** provision a shadow task as above; the platform
  supervises the backend server and gives it a `--8080` preview URL.
- **Fallback (attach by hand):** on a sandbox that wasn't provisioned with the
  backend server, run it in the agent container — works because pod containers
  share localhost, but it's unmanaged (no route/probe, dies with the container):

```bash
bash scripts/shadow/up.sh          # sync env -> build -> run backend on :8080, watch for rebuilds
```

- The **frontend dev server** (`bun run dev`, port 3000) already proxies `/api`
  to `http://localhost:8080` (`apps/frontend/vite.config.ts`). Sidecar servers
  share the pod's localhost, so the running frontend talks to the shadow backend
  with **zero config**. Point your browser at the port-3000 preview URL.
- **Rebuild loop:** edit code, then `bash scripts/shadow/rebuild.sh`. `up.sh`
  watches a trigger file on the shared workspace and restarts the binary.
  Frontend edits hot-reload on their own.
- **Isolation:** the shadow shares the dev Redis, so `sync-env.sh` fences off
  every shared-state consumer it would otherwise contend on:
  - `REDIS_CONSUMER_GROUP`/`REDIS_CONSUMER_ID` → `shadow-<host>` so the SSE and
    generic Redis consumers don't steal the real backend's stream events.
  - `EVENT_HANDLER_ENABLED=false` — the event-handler consumer group is a shared
    constant (`event-handler`), not derived from `REDIS_CONSUMER_GROUP`, so a
    shadow would otherwise split the real backend's event-handler workload
    (webhook→task triggers, message routing). A dev shadow must not act on the
    global event stream.
  - `SANDBOX_IDLE_REAPER_ENABLED=false` + `RECONCILE_*_ENABLED=false` so the
    shadow can never stop/delete/reprovision real sandboxes even if it won the
    (leader-elected) reaper lease.

  It still talks to the **shared dev Postgres/S3**, so be aware you are
  reading/writing real dev data.
- **Extra overrides:** drop `KEY=value` lines in
  `/opt/moatless/data/shadow/overrides.env` (e.g. `RUST_LOG=debug`); they win over
  the synced values.

**Limit:** with the default `moatless-sandbox` SA the shadow backend can serve the
whole API (auth, tasks, repos, messages, SSE, read-only K8s) but **cannot create
new sandbox pods** — that needs Phase 2.

## Fast rebuilds — precompiled dev image

By default `up.sh` runs a cold `cargo build --release` of the whole dependency
graph on every provision (~6–10 min) because the in-tree `target/` lives on the
`workspace-data` mount, which is wiped each provision. `docker/Dockerfile.sandbox-dev`
fixes that: it's a `FROM moatless-sandbox` image that bakes a **warm cargo cache**
— the registry (`/opt/cargo`) plus compiled dependency artifacts
(`/opt/cargo-target`) — both **outside** the `/opt/moatless` mount, so they
survive as image layers. `up.sh` honors `CARGO_TARGET_DIR` (the image sets it to
`/opt/cargo-target`) and so reuses those deps, recompiling only the workspace
crates that actually changed (~1–2 min).

Why it works across a fresh clone: registry crates are keyed by
version/features/rustc/source, **not** by workspace path, so deps compiled at
image-build time (from `/tmp`) are reused when `up.sh` builds from the
provisioned workspace. The workspace crates (`backend`, `crates/*`) still
recompile — that's the "new stuff". (The backend uses only runtime `sqlx::query`,
no compile-time-checked macros, so the image build needs no database.)

CI builds and publishes this automatically as the `moatless-dev` ECR image on
every push that changes the sandbox (the `publish-sandbox-dev` job in
`.github/workflows/cd-ecr.yml` FROMs the just-built sandbox image and reuses the
BuildKit cargo cache across runs). To build it by hand instead:

```bash
# Build + push (single-arch; ops/CI — needs docker + registry creds, a few GB image)
docker build -f docker/Dockerfile.sandbox-dev \
  --build-arg SANDBOX_IMAGE=aorwall/moatless-sandbox:<tag> \
  -t aorwall/moatless-dev:<tag> .
docker push aorwall/moatless-dev:<tag>
```

> Ops prerequisite: the `moatless-dev` ECR repo must exist — add it to the
> moatless service's `ecr_repos` in soap-infra `services.yaml`.

Then point the shadow repo at it so provisioning uses the warm image — set
`dockerImage` on the `moatless-vibe (shadow)` repo (via the API / `moat repo`
config) or add `"dockerImage": "<ecr-registry>/moatless-dev:<tag>"` to
`config.shadow.json`. Without it, everything still works — just with the cold
build. The `backend-server` container inherits the image's `CARGO_TARGET_DIR`
(the curated server-container env doesn't override it), so no config wiring is
needed beyond the image itself.

## Phase 2 — provisioning powers (RBAC only)

Give the shadow backend the real backend's Kubernetes rights via the Helm chart's
opt-in `shadowDev` toggle. It creates a `moatless-dev` ServiceAccount bound to the
same Roles the backend uses (pod-manager, ingressroute-manager, observability):

```yaml
# values.yaml (dev cluster only)
shadowDev:
  enabled: true
  serviceAccount: moatless-dev   # optional; this is the default
```

```bash
helm upgrade moatless-vibe k8s/helm/moatless-vibe -n moatless --set shadowDev.enabled=true
# or: task deploy   (uses the chart)
```

Then run the moatless-vibe dev sandbox under that SA via the **per-repo**
`sandboxServiceAccount` override (already wired end-to-end into `pod_builder.rs`):
- set in `scripts/shadow/config.shadow.json` (`"sandboxServiceAccount": "moatless-dev"`), or
- set on the repository via the API / `moat repo` config.

Only that repo's sandboxes run elevated — every other repo keeps the default SA.
(You *can* instead set it globally with `K8S_SANDBOX_SERVICE_ACCOUNT=moatless-dev`,
but that elevates every sandbox on the cluster.)

## Phase 3 — own the control loop (small Rust change)

`backend/src/sandbox/k8s/pod_builder.rs` currently **hardcodes** the callback URL
it stamps into every sandbox it spawns:

```rust
env_var("BACKEND_GRPC_URL",
    &format!("http://moatless-vibe-backend.{}.svc.cluster.local:{}", ns, grpc_port))
```

So a shadow backend's child sandboxes would call the *real* backend, not the
shadow. To fully act like the backend:

1. Make the advertised URL configurable (e.g. env `BACKEND_ADVERTISE_GRPC_URL` /
   `BACKEND_ADVERTISE_API_URL`), defaulting to today's value. The shadow sets it
   to its own pod/Service address.
2. Expose port `50051` on the shadow sandbox's Service (add it to the server's
   ports) so children can reach it.
3. Allow child → shadow on `50051` in the sandbox `NetworkPolicy`
   (`k8s/helm/moatless-vibe/templates/sandbox-networkpolicy.yaml`).

## Files

- `sync-env.sh` — resolve live backend env + secrets → sourceable env file
- `up.sh` — sync + build + run the backend, watch for rebuilds
- `rebuild.sh` — compile + trigger a restart
- `config.shadow.json` — preview-server config running both servers (copy to `.moatless/config.json`)
- `../../docker/Dockerfile.sandbox-dev` — precompiled image (published by CI as
  `moatless-dev`) with a warm cargo cache so `up.sh` rebuilds fast (point the
  shadow repo's `dockerImage` at it)
- `../../k8s/helm/moatless-vibe/templates/shadow-dev-rbac.yaml` — Phase 2 dev SA +
  RoleBindings, gated on `shadowDev.enabled` in `values.yaml`
