# Moatless Vibe Helm Chart

A Helm chart for deploying Moatless Vibe — an AI-powered development platform.

## Prerequisites

- Kubernetes 1.19+
- Helm 3.0+
- Traefik with its `IngressRoute` CRDs (the app routing and per-sandbox preview
  routes use them)
- PostgreSQL 14+ and Redis 7+ — either the chart's bundled first-party instances
  (dev/testing) or external managed services (production)

## Data stores

The chart can either stand up its own single-node Postgres and Redis, or point at
external managed services.

### Redis (required for event streaming)

| Mode | Values |
|---|---|
| Bundled (default) | `redis.enabled: true`, `redis.url: ""` — deploys a first-party single-node Redis (`redis:7-alpine`, multi-arch) and resolves the backend's `REDIS_URL` to `redis://{release}-redis-master:6379`. |
| External (production) | `redis.enabled: false` with either `redis.urlSecretKey: "REDIS_URL"` (preferred) or `redis.url: "redis://<host>:6379"` — no Redis is deployed. |

The bundled Redis is single-node with an ephemeral data dir — fine for dev, not
for production. For production use a managed Redis (AWS ElastiCache, Azure Cache
for Redis, GCP Memorystore) and set `redis.url`.

### Postgres (required)

| Mode | Values |
|---|---|
| Bundled | `postgres.enabled: true` — deploys a first-party single-node Postgres (`postgres:17-alpine`, multi-arch) and sets the backend's `DATABASE_URL` to the in-cluster Service. |
| External (default) | `postgres.enabled: false` — supply `DATABASE_URL` in the Secret. |

The schema is created automatically at boot by the sqlx migrations compiled into
the backend (`backend/migrations/`), which run and fail-fast on startup. The
bundled Postgres defaults to an ephemeral data dir; set
`postgres.persistence.enabled: true` for a PVC-backed database.

## Secrets

The backend reads three keys from `Secret/{release}-secrets` (or an external
Secret named by `externalSecrets.secretName`):

| Key | Required | Notes |
|---|---|---|
| `DATABASE_URL` | Yes, unless `postgres.enabled: true` | Postgres connection string. With the bundled Postgres the chart sets `DATABASE_URL` for you. |
| `SECRETS_ENCRYPTION_KEY` | Yes | Master key for credential encryption at rest. The backend refuses to boot without it. `openssl rand -base64 32`. |
| `TASK_TOKEN_SECRET` | Yes | HMAC for task-scoped backend↔sandbox JWTs. Every sandbox provision fails without it. `openssl rand -hex 32`. |

Provide the Secret one of three ways:

**Hand-made Secret (recommended):** keep `secrets.create: false` and create it out
of band.

```bash
kubectl create secret generic moatless-secrets \
  --namespace <ns> \
  --from-literal=DATABASE_URL="postgres://user:pass@host:5432/db" \
  --from-literal=SECRETS_ENCRYPTION_KEY="$(openssl rand -base64 32)" \
  --from-literal=TASK_TOKEN_SECRET="$(openssl rand -hex 32)"
# Omit DATABASE_URL when postgres.enabled=true.
```

**Chart-created Secret (testing only):** set `secrets.create: true` and supply
`secrets.databaseUrl`, `secrets.encryptionKey`, `secrets.taskTokenSecret`.

**External secret manager:** set `externalSecrets.enabled: true` and
`externalSecrets.secretName`.

> Agent credentials (`CLAUDE_CODE_OAUTH_TOKEN` / `ANTHROPIC_API_KEY`) are **not**
> chart-installable. They are read only from the encrypted DB, per task, and set
> at runtime via Settings → Secrets. Do not seed a cluster-wide key.
>
> `extraSecretEnv` is an escape hatch for additional secret env vars (e.g. OAuth
> client secrets) — each key is stored in the Secret and injected into the backend.

## Installation

### Dev / testing (bundled Postgres + Redis)

```bash
# Create the app Secret (DATABASE_URL omitted — the bundled Postgres provides it)
kubectl create secret generic moatless-secrets \
  --from-literal=SECRETS_ENCRYPTION_KEY="$(openssl rand -base64 32)" \
  --from-literal=TASK_TOKEN_SECRET="$(openssl rand -hex 32)"

helm install moatless ./k8s/helm/moatless-vibe \
  --set postgres.enabled=true \
  --set redis.enabled=true \
  --set ingress.hosts[0].host=moatless.example.com
```

### Production (external Postgres + Redis)

```bash
kubectl create secret generic vibe-secrets \
  --from-literal=DATABASE_URL="postgres://user:pass@host:5432/db" \
  --from-literal=SECRETS_ENCRYPTION_KEY="$(openssl rand -base64 32)" \
  --from-literal=TASK_TOKEN_SECRET="$(openssl rand -hex 32)"

helm install moatless ./k8s/helm/moatless-vibe \
  --set redis.enabled=false \
  --set redis.url="redis://your-redis-endpoint:6379" \
  --set externalSecrets.enabled=true \
  --set externalSecrets.secretName=vibe-secrets \
  --set ingress.className=alb \
  --set ingress.hosts[0].host=moatless.example.com \
  --set proxyIngress.host="*.preview.example.com"
```

### Local single-node cluster

For a full local bring-up on Docker Desktop Kubernetes (native arm64 images,
first-party Postgres + Redis, `*.localhost` routing through Traefik), use the
committed config and runbook in [`k8s/local/`](../../local/README.md). Install
with:

```bash
helm install moatless ./k8s/helm/moatless-vibe -n moatless -f k8s/local/values-local.yaml
```

## Configuration

### Redis

| Parameter | Description | Default |
|-----------|-------------|---------|
| `redis.enabled` | Deploy the bundled first-party Redis | `true` |
| `redis.url` | External Redis URL stored in Helm values | `""` |
| `redis.urlSecretKey` | Key in the external Secret holding the Redis URL (preferred when `enabled: false`) | `""` |
| `redis.image` | Bundled Redis image | `redis:7-alpine` |
| `redis.maxmemory` | `--maxmemory` cap (paired with `--maxmemory-policy noeviction`) | `768mb` |

### Postgres

| Parameter | Description | Default |
|-----------|-------------|---------|
| `postgres.enabled` | Deploy the bundled first-party Postgres | `false` |
| `postgres.image` | Bundled Postgres image | `postgres:17-alpine` |
| `postgres.auth.username` / `password` / `database` | Credentials + DB the container creates | `moatless` |
| `postgres.service.name` | Service name (the `DATABASE_URL` host) | `postgres` |
| `postgres.persistence.enabled` | PVC-backed data dir instead of emptyDir | `false` |

### Database / Secrets

| Parameter | Description | Default |
|-----------|-------------|---------|
| `database.urlSecretKey` | Secret key holding `DATABASE_URL` | `DATABASE_URL` |
| `secrets.create` | Create the app Secret from values (testing only) | `false` |
| `externalSecrets.enabled` | Use an existing Secret | `false` |
| `externalSecrets.secretName` | Name of the existing Secret | `vibe-secrets` |

### Application

| Parameter | Description | Default |
|-----------|-------------|---------|
| `replicaCount` | Number of legacy Moatless frontend replicas when that UI is deployed | `1` |
| `image.repository` | Legacy Moatless frontend image repository | `aorwall/moatless-frontend` |
| `image.tag` | Legacy frontend and sandbox fallback image tag | `latest` |
| `backend.image.repository` | Backend image repository | `aorwall/moatless-backend` |
| `backend.image.tag` | Backend image tag | `latest` |
| `sandboxImage.repository` | Sandbox image repository | `aorwall/moatless-sandbox` |
| `sandboxImage.pullPolicy` | Sandbox image pull policy | `Always` |

### S3-only EKS Fargate

Fargate sandboxes use API-backed S3 workspace sync and pod-local disk. They do
not mount the S3 CSI shared volume:

```yaml
storage:
  backend: s3
  s3:
    bucketName: moatless-workspaces
    region: eu-north-1
    createSharedVolume: false
kubernetes:
  computeMode: fargate
  sandboxRunAsNonRoot: true
  sandboxEphemeralStorage: 40Gi
  sandboxStorage:
    enabled: false
    defaultMode: s3
  imagePrepull:
    enabled: false
  cacheVolumes: []
  nodeArchive:
    enabled: false
```

Fargate mode also requires equal CPU and memory requests/limits for sandbox and
server containers and rejects per-repository DinD, EBS/node workspaces,
node-local caches, and burstable resource overrides. The EKS VPC CNI does not
enforce Kubernetes NetworkPolicy for Fargate pods, so use this preset only for
trusted repositories.

| Parameter | Description | Default |
|-----------|-------------|---------|
| `storage.s3.createSharedVolume` | Create the Mountpoint S3 CSI shared PV/PVC | `true` |
| `kubernetes.computeMode` | Sandbox compute contract: `standard` or `fargate` | `standard` |
| `kubernetes.sandboxEphemeralStorage` | Sandbox ephemeral-storage request and limit | `""` |

### T3 Code UI

The default front end for the backend, built from
[soaplabs/t3code](https://github.com/soaplabs/t3code) and published as
`aorwall/moatless-t3`. With the default empty `t3.host`, T3 owns the app host
and the chart omits the unused Moatless frontend Deployment and Service.

```bash
helm upgrade --install moatless-vibe ./k8s/helm/moatless-vibe \
  --set t3.host=vibe-t3.example.com
```

That one value does three things: deploys the T3 SPA, gives it a SPA catch-all
to answer, and switches on the backend's T3 compatibility router
(`T3_UI_RPC_ENABLED`) with the T3 origin allow-listed for the WebSocket.
Enabling the UI without the backend router would leave a shell that cannot
bootstrap, so they are not separate switches.

**`t3.host` decides whether the two front ends coexist.** Each owns `/` and
everything under it that is not a backend route, so they cannot share one
origin:

- **`t3.host` set** — T3 gets that hostname, the Moatless SPA keeps the app
  host, and both are reachable. Every backend route is served on both hosts, so
  the T3 client — which derives its HTTP and WebSocket origins from
  `window.location` — needs nothing beyond the hostname resolving here.
- **`t3.host` empty** — T3 replaces the Moatless SPA on the app host.

Setting it to the app host itself is rejected at render time rather than taken
as either meaning: it reads as "give T3 its own origin", so the chart would
leave the Moatless SPA on the app host and add a second catch-all for the same
host at the same priority, and which front end answers `/` would be the
controller's decision rather than the chart's.

Setting `t3.enabled=false` puts the Moatless SPA back; nothing about it is
one-way.

The hostname itself is not the chart's to arrange: it has to resolve to this
deployment's load balancer, and that load balancer's certificate has to cover
it. Session cookies are shared with the app host only when `auth.cookieDomain`
is a parent of both.

Before using it on a shared environment: the T3 UI covers less than the
Moatless UI. The shell, workspace listing and conversation views work; most
other commands are still refused. See
[`docs/t3-code-current-state.md`](../../../docs/t3-code-current-state.md) for
what is actually wired.

| Parameter | Description | Default |
|-----------|-------------|---------|
| `t3.enabled` | Serve the T3 UI; `false` restores the Moatless frontend | `true` |
| `t3.host` | Hostname for the T3 UI; empty replaces the Moatless SPA on the app host | `""` |
| `t3.image.repository` | T3 SPA image repository | `aorwall/moatless-t3` |
| `t3.image.tag` | Image tag | `latest` |
| `t3.replicaCount` | Number of T3 SPA replicas | `1` |
| `t3.extraAllowedOrigins` | Extra origins allowed to open the backend WebSocket | `[]` |

`t3.extraAllowedOrigins` is only needed for a client served from another host —
a desktop or mobile build pointed at this deployment. The origin T3 is served
on is derived from `t3.host` (or `ingress.hosts[0].host`) and `proxy.protocol`,
so it is never configured twice.

The REST API's CSRF check keeps its own list. A cookie-authenticated PATCH/POST
is refused unless its `Origin` is `auth.frontendUrl`, the derived T3 origin
(added whenever `t3.enabled`), or an entry in `auth.csrfTrustedOrigins`.

`t3.extraAllowedOrigins` is **not** inherited by that check, and the two lists
are separate for a reason: this one may carry a wildcard such as
`https://*.preview.moatless.xyz` so sandbox previews can open a socket, and
a preview serves agent-authored code. Trusting it for writes would let a
preview page spend a visitor's session under any `auth.cookieDomain` covering
both. Add a REST origin to `auth.csrfTrustedOrigins`, exactly and by name.

### Kubernetes execution

| Parameter | Description | Default |
|-----------|-------------|---------|
| `kubernetes.enabled` | Enable Kubernetes execution mode | `true` |
| `kubernetes.namespace` | Namespace for sandbox pods | Release namespace |
| `kubernetes.sandboxStorage.enabled` | Global kill switch for per-task PVCs | `true` |
| `kubernetes.sandboxStorage.defaultPvc` | Give each task a durable per-task PVC by default | `true` |
| `kubernetes.sandboxStorage.defaultS3` | Snapshot each task's workspace to S3 by default | `true` |
| `kubernetes.sandboxStorage.coldAfterHours` | Hours stopped before a task's Deployment and PVC are reclaimed (`0` disables) | `24` |
| `kubernetes.sandboxStorage.retainOnClose` | Keep a closed task's S3 workspace snapshot | `true` |
| `kubernetes.nodeArchive.enabled` | Track, retain, and clean node-local archives | `false` |
| `kubernetes.imagePrepull.enabled` | DaemonSet that pre-pulls the sandbox image | `true` |
| `kubernetes.nodeStorageJanitor.enabled` | Per-node cleanup of stale workspace/docker dirs when node archives are enabled | `true` |

#### Claude transcript persistence on S3

Each task's Claude config dir is regenerated on every boot except the conversation
transcripts (`~/.claude/projects/*.jsonl`), which Claude appends to across turns and
which `--resume` reads. The sandbox tars that directory and ships it to S3 **through
the backend** over the existing JWT-authed gRPC (sandbox pods are AWS-credential-less
by design), on a timer and on graceful shutdown, and restores it on the next start.

This persists transcripts off the per-task PVC or node disk and is the
off-node durability path for every storage tier, independent of the workspace's
own `s3Enabled` snapshot.
It reuses the existing uploads bucket and the backend IRSA — no extra bucket,
IAM, or cluster infra. Objects land at
`s3://<uploads-bucket>/<prefix>/<task_id>/transcripts.tar` and are deleted on task
cleanup. Tune the upload cadence with `TRANSCRIPT_SYNC_INTERVAL_SECS` on the sandbox
(default 60s; `0` disables the timer but still flushes on shutdown).

> Scope the uploads bucket's lifecycle expiry to the `uploads/` prefix (or add a
> longer rule for `transcripts/`) so transcripts of long-lived tasks aren't deleted.

#### Choosing a workspace storage tier per repository

Workspace storage is two independent per-repository switches in the repo's
`resources` config, `pvcEnabled` and `s3Enabled`. Each falls back to the
deployment default (`defaultPvc` / `defaultS3`) when the repo leaves it unset,
and a per-repo value always overrides the default. The four combinations:

- **`pvcEnabled: true, s3Enabled: true`** (the default) — a durable per-task PVC
  that also snapshots its git working tree to S3 on stop/remove. On start the
  volume is used as-is when it holds the repo; an empty, redeployed, or
  unattachable volume is restored from S3 instead of stalling. The recommended
  durable tier.
- **`pvcEnabled: true, s3Enabled: false`** — a durable per-task PVC with no S3
  copy. A lost or deleted volume loses the workspace.
- **`pvcEnabled: false, s3Enabled: true`** — a node-local `emptyDir` synced to
  S3 (no PVC, no attach/detach churn on restart), restored on every start.
  Cheaper, but a cold stop drops gitignored build output (`target/`,
  `node_modules/`, …), rebuilt on the next start.
- **`pvcEnabled: false, s3Enabled: false`** — an ephemeral `emptyDir` with no
  persistence; only transcripts survive, via the S3 sync above.

`s3Enabled` requires the uploads bucket to be configured; without it the sync
calls no-op and an S3-only repo behaves as ephemeral. The global
`kubernetes.sandboxStorage.enabled` kill switch still wins over `pvcEnabled`.

Optionally cap a workspace's size with `storage` (it sizes the PVC, or the
`emptyDir` when there is no PVC).

#### The cold tier: reclaiming idle claims

Stopping a task scales its Deployment to zero and keeps the PVC, so the next
start is warm. That is the right trade for a task someone will come back to
this afternoon, and the wrong one for a fleet: a `gp3` volume is billed on its
provisioned size whether or not a pod is attached, so at any real scale the
idle claims — not the running pods — are the bill.

`kubernetes.sandboxStorage.coldAfterHours` (default `24`) adds a third
transition to the reaper. Once a task has been stopped that long, the backend
deletes its Deployment and its PVC and leaves the S3 snapshot as the only copy.
The task stays open and reads as Stopped; a later start provisions a fresh
volume, restores the snapshot into it, and re-runs the repo's setup commands to
rebuild gitignored output. What the cold tier costs is a slower first start.
What it buys is object-storage pricing on an idle workspace instead of
block-storage pricing.

The volume is deleted only once the snapshot is proven to be newer than the
stop. When that cannot be shown — the pre-stop upload failed, or S3 cannot be
read — the backend first runs a short-lived rescue pod that mounts the volume
and uploads it, and abandons the transition entirely if that fails. An
unreadable bucket never becomes a deletion.

Two things to set alongside it:

- **Keep `defaultS3` on.** With no snapshot there is nothing to reclaim toward,
  and the transition never fires. It is inert rather than dangerous, but it is
  also useless.
- **Give the bucket a lifecycle rule.** Nothing here ever deletes a snapshot,
  and `retainOnClose` (default `true`) means closing a task does not either —
  the reaper closes tasks after 7 days of inactivity, and a machine-initiated
  close is a poor moment to destroy someone's work. Expire the
  `<workspacePrefix>/` prefix on whatever horizon you actually want to keep
  abandoned workspaces for.

Set `coldAfterHours: 0` to switch the tier off; a per-task
`tasks.cold_after_hours` overrides the deployment default for one task.

Docker-in-Docker storage is controlled separately with `dockerStorageMode`.
For repos with `dockerAccess`, `dockerStorageMode: "node"` stores
`/var/lib/docker` on best-effort node-local hostPath storage so Docker layers,
images, and volumes can survive stop/start on the same node. Use
`dockerStorageMode: "emptyDir"` plus `dockerStorage` when you need a hard
Kubernetes size cap instead.

Set `kubernetes.nodeArchive.enabled=true` (environment variable
`K8S_SANDBOX_NODE_ARCHIVE_ENABLED=true`) to manage node storage across
lifecycles. The backend then records scheduled nodes and operation generations
in a deterministic task ConfigMap. Retained starts pin to the newest recorded
surviving node; hard deletion runs a deterministic cleanup pod on every
recorded node before deleting the metadata.

This opt-in also renders the namespace-scoped ConfigMap RBAC, read-only node
ClusterRole, and—when `kubernetes.nodeStorageJanitor.enabled=true`—the janitor
DaemonSet. The janitor removes old task directories only when both their
Deployment and retained-archive ConfigMap are absent. When node archives are
disabled, no janitor is deployed and stale hostPath directories require
operator cleanup.

Declaratively via `.moatless/config.json`, then sync:

```json
{
  "resources": {
    "pvcEnabled": true,
    "s3Enabled": true,
    "storage": "30Gi",
    "dockerStorageMode": "node",
    "dockerStorage": "30Gi"
  }
}
```

```bash
moat repos sync-config <repo-id>
```

Imperatively via the CLI (a resources update **replaces** the whole `resources`
object, so pass all resource flags you want to keep together):

```bash
moat repos update <repo-id> --storage 30Gi --docker-storage-mode node --docker-storage 30Gi
```

Or via the API:

```bash
PATCH /api/.../repositories/<repo-id>   { "resources": { "pvcEnabled": true, "s3Enabled": true, "dockerStorageMode": "node" } }
```

### Shadow-Dev (running the app inside a sandbox)

Opt-in identity for developing moatless-vibe **inside a sandbox** ("shadow mode"):
run the backend + frontend in a sandbox pod and use it like the real backend,
without building images. Since a sandbox is already a pod in the cluster, the only
thing it lacks is the backend's Kubernetes RBAC. Enabling `shadowDev` creates a
`moatless-dev` ServiceAccount bound to the same Roles the backend uses
(pod-manager, ingressroute-manager, observability) so a shadow backend can
provision sandboxes.

| Parameter | Description | Default |
|-----------|-------------|---------|
| `shadowDev.enabled` | Create the shadow-dev ServiceAccount + RoleBindings (requires `rbac.create`) | `false` |
| `shadowDev.serviceAccount` | Name of the shadow-dev ServiceAccount (in the sandbox namespace) | `moatless-dev` |
| `shadowDev.serviceAccountAnnotations` | Annotations for the SA (e.g. EKS Pod Identity / IRSA) | `{}` |

```bash
# Enable on a dev cluster
helm upgrade moatless-vibe k8s/helm/moatless-vibe -n moatless --set shadowDev.enabled=true
```

Then opt one workspace into that identity — **admin-only**, like every other
privileged sandbox identity:

```bash
moat workspaces update   <workspace-id> --sandbox-service-account moatless-dev
# ...or `override` instead of `update` if the workspace came from git (see below)
```

Only that workspace's sandboxes run elevated. See `scripts/shadow/README.md`
for the full flow.

> `sandboxServiceAccount` declared in `.moatless/config.json` or
> `.moatless/workspaces.json` is **ignored** — git-sync strips it and logs a
> warning naming the file. Repo files are writable by anyone with commit
> access, which is a wider set than "admin", so honouring them would bypass the
> gate. Set it through the API instead.

> Dev clusters only. The field just *selects* the SA — RBAC (these bindings) is
> what grants power, so leave `shadowDev.enabled=false` in production.

### Sandbox access (using the cluster from a sandbox)

By default a sandbox cannot talk to the Kubernetes API at all, for two
independent reasons — and a workspace needs **both** lifted:

| Gate | What blocks it | How a workspace opts in |
|------|----------------|-------------------------|
| Network | The kube-apiserver ClusterIP (e.g. `172.20.0.1`) is inside `172.16.0.0/12`, which the base sandbox policy lists under `except:`. `kubectl` hangs on connect. | `networkProfile` = the profile's **map key** |
| RBAC | The Roles in `rbac.yaml` bind the *backend* SA, not the sandbox one — so `kubectl get ns` would 403 even with a network path. | `sandboxServiceAccount` = the profile's `serviceAccount` |

Without this, cluster-audit tasks reconstruct state from
Prometheus/kube-state-metrics, which exposes **neither NetworkPolicy objects nor
namespace labels**.

`sandboxAccess.profiles` is a map; the key is the pod label, so the policy
selector can never drift from the profile name. `access` picks a **chart-owned**
RBAC shape rather than putting raw rules in values — those rules decide who can
read your Secrets, so they stay where code review sees them, and a chart upgrade
can improve them without every install pinning a stale copy in gitops.

| Preset | Scope | Grants |
|--------|-------|--------|
| `cluster-read` | Cluster-wide ClusterRole | `get`/`list`/`watch`. **Secrets excluded by construction.** |
| `namespace-admin` | Role + RoleBinding in the app **and** sandbox namespaces | Full verbs — read Secrets, delete, patch (`kubectl rollout restart`). Namespaced on purpose: the blast radius stops at those two namespaces. |

| Parameter | Description | Default |
|-----------|-------------|---------|
| `sandboxAccess.apiServerCidrs` | kube-apiserver ClusterIP(s), shared by all profiles. **Required once any profile is enabled** | `[]` |
| `sandboxAccess.apiServerPort` | kube-apiserver port | `443` |
| `sandboxAccess.profiles.<key>.enabled` | Create this profile's SA, RBAC and NetworkPolicy | `false` |
| `sandboxAccess.profiles.<key>.access` | `cluster-read` or `namespace-admin` | — |
| `sandboxAccess.profiles.<key>.serviceAccount` | SA name, in the sandbox namespace | — |
| `sandboxAccess.profiles.<key>.serviceAccountAnnotations` | SA annotations (EKS Pod Identity / IRSA) | `{}` |
| `sandboxAccess.profiles.<key>.extraRules` | Extra RBAC rules appended verbatim (environment CRDs) | `[]` |
| `sandboxAccess.profiles.<key>.extraEgress` | Extra egress appended verbatim (databases, other in-VPC destinations) | `[]` |

```yaml
# apiServerCidrs from: kubectl -n default get svc kubernetes -o jsonpath='{.spec.clusterIP}'
sandboxAccess:
  apiServerCidrs: ["172.20.0.1/32"]
  profiles:
    cluster-reader:
      enabled: true
      access: cluster-read
      serviceAccount: moatless-cluster-reader
      extraRules:
        - apiGroups: ["argoproj.io", "cilium.io", "traefik.io"]
          resources: ["*"]
          verbs: ["get", "list", "watch"]
    admin:
      enabled: true
      access: namespace-admin
      serviceAccount: moatless-admin
      extraEgress:                     # so `psql` works while troubleshooting
        - to: [{ ipBlock: { cidr: 10.30.20.0/24 } }]
          ports: [{ protocol: TCP, port: 5432 }]
```

Then opt one workspace into one profile — nothing else changes:

```bash
# Admin-only: the API rejects both fields for non-admin callers with 403.
moat workspaces update <workspace-id> \
  --sandbox-service-account moatless-cluster-reader \
  --network-profile cluster-reader

# …or the admin profile, to read Secrets and restart workloads:
moat workspaces update <workspace-id> \
  --sandbox-service-account moatless-admin \
  --network-profile admin
```

`update` only accepts a **manual** workspace. One synced from a repo's
`.moatless/workspaces.json` has `source=git` and is read-only, so `update`
returns 409 — use `moat workspaces override` with the same flags. That sets
`source=manual`, which takes the workspace out of git sync until you hand it
back with `moat workspaces reset-git` (and that re-sync drops the identity
fields again, since git-declared values are never honoured).

`sandboxServiceAccount` and `networkProfile` are what actually inherit a
profile, so **every** write path restricts them — otherwise enabling a profile
here would make it self-serve for anyone who can create a workspace, and "point
one workspace at it" would be advice rather than a control:

| Path | Behaviour |
|------|-----------|
| `POST /workspaces`, `PATCH /workspaces/{id}`, `POST /workspaces/{id}/override` | 403 for non-admins, via `handlers.rs::ensure_privileged_sandbox_fields`. Only fires when the request carries a field, so ordinary non-admin edits are unaffected. |
| Git sync — `.moatless/workspaces.json`, `.moatless/config.json` | Both fields stripped and a warning logged naming the file, via `git_sync.rs::strip_git_declared_identity`. Repo files are writable by anyone with commit access, a wider set than "admin". |

Taking ownership of a git-synced workspace through `override` is deliberately
not a way around this: that endpoint skips the mutability check but keeps the
privilege gate.

Verify from inside a sandbox of that workspace:

```bash
kubectl get ns                     # was a 45s connect timeout before
kubectl get networkpolicies -A
```

Adding a third identity is a values entry when it reuses a preset; only a
genuinely new RBAC *shape* needs a chart change. An unrecognised `access` value
fails the render rather than creating an SA with no RBAC, and enabling any
profile without `apiServerCidrs` fails too — otherwise you would deploy
permissions that cannot reach the API server and get the same silent timeout
this feature exists to remove.

> **`namespace-admin` is a large grant.** The app namespace holds the backend's
> own Secrets — database credentials, provider API keys, session signing keys —
> so enabling it gives an LLM agent with internet egress read access to all of
> them, plus delete on anything running there. Troubleshooting clusters only:
> point exactly one workspace at it and revoke when done. Use `cluster-read`
> when read-only inspection is enough.

> Keep profile keys out of `egressProxy.profiles`. A sealed profile is excluded
> from the base sandbox policy, so those pods would get API-server access and
> lose general internet egress. The chart fails the render on that combination
> rather than leaving it as a runtime surprise.

### Ingress Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `ingress.enabled` | Emit the main Ingress object | `true` |
| `ingress.hosts[0].host` | App hostname (also read by the Traefik IngressRoutes) | `moatless.example.com` |
| `proxy.traefikIngressRoute.enabled` | Emit the app's Traefik IngressRoute CRDs | `false` |
| `proxyIngress.enabled` | Emit the wildcard preview Ingress | `true` |
| `proxyIngress.host` | Wildcard host for previews | `*.preview.example.com` |

> The Traefik IngressRoutes read the app host from `ingress.hosts[0].host`
> regardless of `ingress.enabled`. Keep `ingress.hosts[0].host` set even when you
> disable the Ingress object, or the routes fall back to `moatless.example.com`.

## Redis Streams configuration

The application uses Redis Streams for event delivery:

- **Stream naming**: `task-events:{taskId}` (one stream per task)
- **Consumer group**: `main-app-consumers` (configurable via `REDIS_CONSUMER_GROUP`)
- **Stream trimming**: ~10,000 messages per stream (`REDIS_MAX_STREAM_LENGTH`)
- **Blocking reads**: 5-second timeout (`REDIS_BLOCK_MS`)

## Monitoring

Application metrics are exposed on the backend `metrics` port (9090) at `/metrics`
and discovered via the `prometheus.io/*` pod annotations. Enable the bundled
`PodMonitor`/`PrometheusRule` with `monitoring.prometheus.enabled: true` (requires
a Prometheus Operator).

## Troubleshooting

### Redis connection issues

Test connectivity from a throwaway pod:

```bash
kubectl run redis-test --rm -it --image=redis:7-alpine -- \
  redis-cli -u "redis://<release>-redis-master:6379" PING
```

### Event delivery issues

```bash
redis-cli KEYS "task-events:*"
redis-cli XINFO GROUPS task-events:{taskId}
redis-cli XPENDING task-events:{taskId} main-app-consumers
kubectl logs -l app.kubernetes.io/component=backend --tail=100
```

### Backend won't boot / API calls fail

`/health` is exempt from most checks, so a green pod is not proof of a working
config. Confirm migrations actually ran:

```bash
kubectl logs deploy/<release>-backend | grep "Database migrations complete"
```

A missing `SECRETS_ENCRYPTION_KEY` exits the backend at boot; a bad `DATABASE_URL`
does **not** crash the pod but leaves every API call failing.

## Upgrading

```bash
helm upgrade moatless ./k8s/helm/moatless-vibe \
  --reuse-values \
  --set image.tag=v0.2.0
```

## Uninstalling

```bash
helm uninstall moatless
# Bundled Postgres with persistence enabled leaves a PVC behind:
kubectl delete pvc -l app.kubernetes.io/name=postgres
```

## Support

- GitHub Issues: https://github.com/aorwall/moatless-vibe/issues
- Documentation: https://github.com/aorwall/moatless-vibe/tree/main/docs
