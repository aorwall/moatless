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

## Phase 1 — shadow app (works with today's sandbox permissions)

No code changes, no cluster changes. The sandbox SA can already `get deployments`
and `get secrets`, and `cargo` + `bun` are in the sandbox image.

```bash
# From the repo root inside the sandbox:
bash scripts/shadow/up.sh          # sync env -> build -> run backend on :8080, watch for rebuilds
```

- The **frontend dev server** (`bun run dev`, port 3000) already proxies `/api`
  to `http://localhost:8080` (`apps/frontend/vite.config.ts`). Sidecar servers
  share the pod's localhost, so the running frontend talks to the shadow backend
  with **zero config**. Point your browser at the port-3000 preview URL.
- **Rebuild loop:** edit code, then `bash scripts/shadow/rebuild.sh`. `up.sh`
  watches a trigger file on the shared workspace and restarts the binary.
  Frontend edits hot-reload on their own.
- **Isolation:** `sync-env.sh` overrides `REDIS_CONSUMER_GROUP`/`REDIS_CONSUMER_ID`
  to a `shadow-<host>` id so the shadow does **not** steal stream events from the
  real backend's consumer group. It talks to the **shared dev Postgres/S3**, so
  be aware you are reading/writing real dev data.
- **Extra overrides:** drop `KEY=value` lines in
  `/opt/moatless/data/shadow/overrides.env` (e.g. `RUST_LOG=debug`); they win over
  the synced values.

**Limit:** with the default `moatless-sandbox` SA the shadow backend can serve the
whole API (auth, tasks, repos, messages, SSE, read-only K8s) but **cannot create
new sandbox pods** — that needs Phase 2.

### Wiring it into the preview-server config

`scripts/shadow/config.shadow.json` declares both servers (frontend + shadow
backend) so a dedicated dev task brings them up automatically. Use it by copying
it to `.moatless/config.json` on a `moatless-vibe (shadow)` repo/branch. It is
intentionally **not** the default — the backend sidecar compiles Rust on start.

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
- `../../k8s/helm/moatless-vibe/templates/shadow-dev-rbac.yaml` — Phase 2 dev SA +
  RoleBindings, gated on `shadowDev.enabled` in `values.yaml`
