# Shadow backend — mirrord-style dev, without mirrord

Run the moatless **backend inside a moatless sandbox** and use it
like the real backend. mirrord exists to bridge a laptop *into* a cluster; a
sandbox is **already a pod in the dev cluster**, so the hard parts (outgoing
traffic, DNS, cluster identity) are free or native. This harness reproduces the
rest.

## What maps to what

| mirrord capability | Native equivalent here |
| --- | --- |
| Outgoing traffic + DNS (reach Postgres, K8s API, sandbox ClusterIPs) | **Free** — the sandbox is on the pod network already |
| Env / secret sync from the target | `sync-env.sh` reads the live backend Deployment env + resolves `moatless-secrets` |
| Copy the target's ServiceAccount token (RBAC to create pods) | Helm `shadowDev` toggle (`templates/shadow-dev-rbac.yaml`) binds a `moatless-dev` SA to the backend's existing Roles |
| Steal incoming traffic | Not needed for HTTP: the backend has its own `--8080` preview URL, and sibling containers reach `localhost:8080`; for sandbox gRPC callbacks the backend self-advertises its address (Phase 3) |

## How the shadow backend actually runs (mechanism)

The shadow backend runs as a **Script** — a named command a Workspace declares
(`ScriptConfig`: name, command, optional label/icon/port/envVars/runOnStart),
executed as a terminal session inside the sandbox's own container, not a
separate container:

- `crates/t3code/src/scripts.rs::run` execs the Script's `command` into a named
  terminal session in the sandbox, publishing its `port` first if it has one.
- A Script's `port` gets a Traefik IngressRoute `{task_id}--{port}.{proxy_domain}`
  (`crates/sandbox-k8s/src/ingress.rs`), reached via a Service port the
  `SandboxPortPublisher` trait creates on the fly (`crates/sandbox-k8s/src/edge.rs`,
  named `scr-{port}`) — the backend on 8080 gets the `…--8080.preview…` URL.
  Docker's `SandboxPortPublisher` always returns `None`, so a Docker sandbox's
  Script never gets a preview URL.
- All pod containers share the network namespace, so a sibling container
  reaches the backend on `localhost:8080` regardless of whether its port is
  published.
- `scripts` come from the workspace DB record, synced from
  **`.moatless/workspaces.json`** (`crates/workspaces/src/workspace/git_sync.rs`).

So the shadow backend should be declared as a **Script** with
`command: "bash scripts/shadow/up.sh"`. Put it on a **dedicated Workspace** —
**not** whatever workspace the shared/global `moatless` repo already uses.
Declaring the `backend` Script there would make **every** moatless task
(agents included) compile Rust on start. Keep it isolated: create a dedicated
repository record for the git connection, then a dedicated workspace carrying
the Script:

```bash
# User-scoped dev repo pointing at the same GitHub repo (only you use it) —
# a repository record is only a git connection; run-configuration lives on
# the workspace, set separately below:
REPO_ID=$(moat repos create --name "moatless (shadow)" \
  --git-url https://github.com/soaplabs/moatless --branch main \
  --json | jq -r '.id')

moat workspaces create --name "moatless (shadow)" \
  --docker-access true \
  --docker-image <ecr-registry>/moatless-dev:latest \
  --setup-commands-json '[
    "curl -fsSL https://bun.sh/install | BUN_INSTALL=/opt/moatless/data/.bun bash",
    "/opt/moatless/data/.bun/bin/bun install"]' \
  --scripts-json '[
    {"name":"backend","label":"Shadow Backend","command":"bash scripts/shadow/up.sh",
     "port":8080,"runOnStart":true}]' \
  --resources-json '{"storage":"40Gi","s3Enabled":true}' \
  --repos-json "[{\"repositoryId\":\"$REPO_ID\",\"isPrimary\":true}]"
```

The elevated identity is **not** part of that command — `sandboxServiceAccount`
lives on the workspace, is admin-only, and has no `moat repos` flag. Grant it
separately once the repo's workspace exists (see
[Phase 2](#phase-2--provisioning-powers-rbac-only)):

```bash
# Manual workspace:
moat workspaces update   <workspace-id> --sandbox-service-account moatless-dev
# Workspace synced from .moatless/workspaces.json (source=git) — `update` 409s:
moat workspaces override <workspace-id> --sandbox-service-account moatless-dev
```

Check which you have with `moat workspaces get <workspace-id>`; `override` is
the one that takes a git-synced workspace, and it flips `source` to `manual` so
git sync leaves it alone from then on. See
[Phase 2](#phase-2--provisioning-powers-rbac-only) for the full caveat.

`workspaces.shadow.json` is the equivalent as a committed
`.moatless/workspaces.json` if
you'd rather wire it via a shadow branch — for the Script and setup commands
only; it cannot carry the SA. Provisioning a task on the shadow workspace runs
the `backend` Script as a terminal session in the sandbox (start it with
`moat tasks scripts run backend <task-id>`, or let `runOnStart` do it; read its
output in that terminal in the T3 UI, not with `moat tasks logs`). Its API
answers on the `--8080` preview URL.

> Declared the `backend` Script on the wrong workspace by mistake? Revert with
> `moat repos sync-config <repo_id>` (restores `scripts` from `main`), and clear
> the SA directly — the update API can't null it —
> `UPDATE repositories SET sandbox_service_account=NULL WHERE id='<repo_id>';`.

## Phase 1 — shadow app (works with today's sandbox permissions)

No cluster changes. The sandbox SA can already `get deployments` and
`get secrets`, and `cargo` + `bun` are in the sandbox image.

- **Preferred (declared Script):** provision a shadow task as above; the
  `backend` Script runs in the sandbox and, under Kubernetes, gets a `--8080`
  preview URL.
- **Fallback (attach by hand):** on a sandbox that wasn't provisioned with the
  `backend` Script, run it in the agent container — works because pod
  containers share localhost, but it's unmanaged (no port publishing, dies
  with the container):

```bash
bash scripts/shadow/up.sh          # sync env -> build -> run backend on :8080, watch for rebuilds
```

- **Browser client:** T3 Code is the UI. Its dev server proxies `/api` and `/ws`
  to the shadow backend on `localhost:8080`; the `t3-development` skill covers
  that paired setup.
- **Rebuild loop:** edit code, then `bash scripts/shadow/rebuild.sh`. `up.sh`
  watches a trigger file on the shared workspace and restarts the binary.
- **Isolation:** the shadow shares the dev Postgres, so `sync-env.sh` fences off
  every shared-state consumer it would otherwise contend on:
  - `EVENT_HANDLER_ENABLED=false` — every replica claims off one shared dispatch
    queue, so a shadow would otherwise split the real backend's event-handler
    workload (webhook→task triggers, message routing). A dev shadow must not act
    on the global event stream. Its SSE feed needs no such flag: that cursor is
    per-process and in memory, so it reads the feed without claiming from it.
  - `SANDBOX_IDLE_REAPER_ENABLED=false` so the shadow can never stop or delete
    real sandboxes even if it won the (leader-elected) reaper lease.
  - `RUN_MIGRATIONS=false` — the shadow shares the real dev Postgres, so it must
    not apply pending migrations (that would alter the schema the real backend
    depends on). It runs against whatever schema is already there. Need new
    schema? Point `DATABASE_URL` at your own throwaway DB in `overrides.env` and
    set `RUN_MIGRATIONS=true` there.

  It still talks to the **shared dev Postgres/S3**, so be aware you are
  reading/writing real dev data.
- **Local browser origins:** `T3_UI_RPC_ALLOWED_ORIGINS` is synced from the live
  Deployment, which admits only the preview domain. A browser on
  `http://localhost:5733` then fails the UI-RPC WebSocket upgrade with **403**
  while HTTP through the vite proxy keeps working — so the T3 app renders a
  convincing empty "No projects yet" state instead of an auth error.
  `sync-env.sh` keeps the synced value and appends
  `http://localhost:5733,http://127.0.0.1:5733`. Widening it here touches only
  the shadow; the real backend's allowlist is untouched.
- **Extra overrides:** drop `KEY=value` lines in
  `$RUNTIME_DIR/shadow/overrides.env` (e.g. `RUST_LOG=debug`); they win over
  the synced values. `sync-env.sh` reads no other path: an override file under
  `DATA_DIR`, the shared-data volume, is ignored.
  `RUNTIME_DIR` (default `/run/moatless`) may be `tmpfs`, so a hand-written
  `overrides.env` there might not survive a sandbox restart the way a
  regenerated `backend.env` does — keep a copy outside the sandbox if you'd
  otherwise have to retype it.

**Limit:** with the default `moatless-sandbox` SA the shadow backend can serve the
whole API (auth, tasks, repos, messages, SSE, read-only K8s) but **cannot create
new sandbox pods** — that needs Phase 2.

## Build profile — `SHADOW_CARGO_PROFILE`

Both `up.sh` and `rebuild.sh` build with `--profile "${SHADOW_CARGO_PROFILE:-release}"`.
The default is unchanged (`release`, what the dev image bakes), but the release
profile is a poor fit for an edit→run loop. Measured in a sandbox, one edit
under `backend/src/ui_rpc/`:

| `SHADOW_CARGO_PROFILE` | Rebuild after one backend edit | Binary lands in |
| --- | --- | --- |
| `release` (default) | **4 m 34 s** | `target/release/` |
| `ci` (release opts, `lto = false`, `codegen-units = 16`) | in between | `target/ci/` |
| `dev` | **21 s** | `target/debug/` |

The release profile also shares **no** artifacts with the dev profile that
`cargo test` and `cargo clippy` build, so iterating on the backend while
running checks costs two independent builds of the same crate. `dev` collapses
that to one. That is a disk problem as well as a time one: a sandbox on the
default per-repo workspace gets a 10Gi `workspace-data` EmptyDir, and a
`target/` holding both profiles overruns it — `Usage of EmptyDir volume
"workspace-data" exceeds the limit "10Gi"` evicts the pod mid-build. Run Rust
work on a workspace with real storage configured, and keep `CARGO_TARGET_DIR`
off that volume when you only need a throwaway build.

```bash
export SHADOW_CARGO_PROFILE=dev
bash scripts/shadow/rebuild.sh
```

> Set it in **both** places or not at all: the `backend` Script's env (so
> `up.sh` runs the right binary) *and* your shell (so `rebuild.sh` compiles the
> one it runs). Mismatched, `rebuild.sh` builds a profile `up.sh` never starts
> and the backend silently keeps serving the old code. The baked-binary
> fast path only applies to `release`; any other profile always builds.

## Fast rebuilds — precompiled dev image

By default a fresh provision pays a cold `cargo build --release` of the whole
dependency graph (~6–10 min) *plus* a cold `bun install` — both because the
in-tree `target/` and `node_modules/` live on the `workspace-data` mount, which
is wiped each provision. `docker/Dockerfile.sandbox-dev` fixes that: it's a
`FROM moatless-sandbox` image that bakes **three warm caches**, all **outside**
the `/opt/moatless` mount so they survive as image layers:

- `/opt/cargo-target` — compiled dependency artifacts plus the baked backend
  binary. `up.sh` honors `CARGO_TARGET_DIR` (the image sets it here). If the
  provisioned checkout is exactly the image-build commit and Rust workspace
  paths are clean, it starts the baked binary immediately. After code edits, it
  falls back to rebuilding with the warm target and recompiles only changed
  workspace crates.
- `/opt/cargo/registry` — downloaded crate sources (`CARGO_HOME`). Without this
  the provision-time build re-downloads the whole index + sources before it can
  compile even with a warm target.
- `/opt/bun-cache` — bun's global install cache (the image sets
  `BUN_INSTALL_CACHE_DIR` here). The provision-time `bun install` setup command
  then links `node_modules` from cache offline instead of downloading.

Why it works across a fresh clone: registry crates are keyed by
version/features/rustc/source, **not** by workspace path, so deps compiled at
image-build time (from `/tmp`) are reused when `up.sh` builds from the
provisioned workspace; likewise bun's cache is content-addressed, not path-bound.
The workspace crates (`backend`, `crates/*`) only recompile after the checkout
diverges from the image-build commit, and `node_modules` is still created fresh
in the workspace (just from the warm cache). (The backend uses only runtime
`sqlx::query`, no
compile-time-checked macros, so the image build needs no database.)

CI publishes this as the `moatless-dev:latest` ECR image via an **on-demand**
job — the `Build dev sandbox image` workflow
(`.github/workflows/build-dev-image.yml`), run manually from the Actions tab. It
FROMs the `moatless-sandbox` image (tag configurable, defaults to `latest`) and
reuses the BuildKit cargo cache across runs. It is intentionally *not* part of
the push-to-main flow — the image is a heavy multi-GB build used only by the
opt-in shadow repo, so rebuild it by hand when the base sandbox image or the
shadow-dev dependency set has meaningfully changed. To build it locally instead:

```bash
# Build + push (single-arch; ops/CI — needs docker + registry creds, a few GB image)
docker build -f docker/Dockerfile.sandbox-dev \
  --build-arg SANDBOX_IMAGE=aorwall/moatless-sandbox:<tag> \
  -t aorwall/moatless-dev:<tag> .
docker push aorwall/moatless-dev:<tag>
```

> Ops prerequisite: the `moatless-dev` ECR repo must exist — add it to the
> moatless service's `ecr_repos` in soap-infra `services.yaml`.

Then point the shadow workspace at it so provisioning uses the warm image —
`moat workspaces update <workspace-id> --docker-image <ecr-registry>/moatless-dev:latest`
(or add `"dockerImage": "<ecr-registry>/moatless-dev:latest"` to the workspace
entry in `workspaces.shadow.json`). Without it, everything still works — just
with the cold build. The `backend` Script runs in that same sandbox container,
so it inherits the image's `CARGO_TARGET_DIR` directly — no separate config
wiring needed beyond the image itself.

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
helm upgrade moatless k8s/helm/moatless -n moatless --set shadowDev.enabled=true
# or: task deploy   (uses the chart)
```

Then run the shadow sandbox under that SA via the **per-workspace**
`sandboxServiceAccount` override (wired end-to-end into `pod_builder.rs`):

```bash
# Admin-only — the API returns 403 otherwise.
# `update` is for manual workspaces; it 409s on a git-synced one.
moat workspaces update   <workspace-id> --sandbox-service-account moatless-dev

# A workspace declared in .moatless/workspaces.json has source=git, so use:
moat workspaces override <workspace-id> --sandbox-service-account moatless-dev
```

Only that workspace's sandboxes run elevated — every other workspace keeps the
default SA. (You *can* instead set it globally with
`K8S_SANDBOX_SERVICE_ACCOUNT=moatless-dev`, but that elevates every sandbox on
the cluster.)

> **`override` is a one-way door until you reclaim it.** It sets `source=manual`
> so git sync stops updating that workspace — later edits to
> `.moatless/workspaces.json` no longer reach it, and you re-apply them by hand.
> `moat workspaces reset-git <workspace-id>` hands it back to git, but that
> re-syncs from the committed config, which means the SA is stripped again
> (git-declared identity is never honoured) and you have to re-`override` to
> restore it.

> **Not settable from git.** `sandboxServiceAccount` in
> `.moatless/workspaces.json` is stripped on sync
> (`git_sync.rs::strip_git_declared_identity`), logged, and reported in the
> sync summary. Repo files are writable by anyone with commit access, a wider set
> than "admin", so honouring them would bypass the gate. There is likewise no
> `moat repos --sandbox-service-account` flag — the field lives on the
> workspace, not the repository.

## Phase 3 — own the control loop (small Rust change)

`crates/sandbox-k8s/src/pod_builder/containers.rs` stamps the callback URL into
every sandbox it spawns as `http://<K8S_BACKEND_SERVICE_HOST>.<namespace>.svc…`,
and `sync-env.sh` copies the real backend's `K8S_BACKEND_SERVICE_HOST`. So a
shadow backend's child sandboxes would call the *real* backend, not the
shadow. To fully act like the backend:

1. Override `K8S_BACKEND_SERVICE_HOST` in the shadow's env with a Service that
   routes to the shadow pod.
2. Expose port `50051` on the shadow sandbox's Service so children can reach it.
3. Allow child → shadow on `50051` in the sandbox `NetworkPolicy`
   (`k8s/helm/moatless/templates/sandbox-networkpolicy.yaml`).

## Files

- `sync-env.sh` — resolve live backend env + secrets → sourceable env file, by
  default `$RUNTIME_DIR/shadow/backend.env` (`/run/moatless/shadow/backend.env`).
  It refuses a target under `DATA_DIR` or `WORKSPACE_DIR`: the file holds
  `TASK_TOKEN_SECRET`, `DATABASE_URL` and ~135 other secrets, and every sandbox
  mounts those two directories. Override with `SHADOW_ENV_FILE`.
- `up.sh` — sync + build + run the backend, watch for rebuilds
- `rebuild.sh` — compile + trigger a restart
- `workspaces.shadow.json` — workspace declaring the backend Script (copy to `.moatless/workspaces.json`)
- `../../docker/Dockerfile.sandbox-dev` — precompiled image (published on demand
  as `moatless-dev:latest` by the `Build dev sandbox image` workflow) with warm
  caches so `up.sh` rebuilds fast (point the shadow repo's `dockerImage` at it)
- `../../.github/workflows/build-dev-image.yml` — on-demand (workflow_dispatch)
  job that builds + pushes `moatless-dev:latest`
- `../../k8s/helm/moatless/templates/shadow-dev-rbac.yaml` — Phase 2 dev SA +
  RoleBindings, gated on `shadowDev.enabled` in `values.yaml`
