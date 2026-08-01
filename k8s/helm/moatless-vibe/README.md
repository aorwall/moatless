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
| External (production) | `redis.enabled: false`, `redis.url: "redis://<host>:6379"` — no Redis is deployed; the backend uses the given URL. |

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
  --set ingress.hosts[0].host=vibe.example.com
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
  --set ingress.hosts[0].host=vibe.example.com \
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
| `redis.url` | External Redis URL (used when set; required when `enabled: false`) | `""` |
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
| `replicaCount` | Number of frontend replicas | `1` |
| `image.repository` | Frontend (nginx) image repository | `aorwall/moatless-frontend` |
| `image.tag` | Image tag | Chart `appVersion` |
| `backend.image.repository` | Backend image repository | `aorwall/moatless-backend` |
| `sandboxImage.repository` | Sandbox image repository | `aorwall/moatless-sandbox` |

### T3 Code UI

An alternative front end for the same backend, built from
[soaplabs/t3code](https://github.com/soaplabs/t3code) and published as
`aorwall/moatless-t3`.

```bash
helm upgrade --install moatless-vibe ./k8s/helm/moatless-vibe \
  --set t3.enabled=true \
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

Before enabling it on a shared environment: the T3 UI covers less than the
Moatless UI. The shell, workspace listing and conversation views work; most
other commands are still refused. See
[`docs/t3-code-current-state.md`](../../../docs/t3-code-current-state.md) for
what is actually wired.

| Parameter | Description | Default |
|-----------|-------------|---------|
| `t3.enabled` | Serve the T3 UI | `false` |
| `t3.host` | Hostname for the T3 UI; empty replaces the Moatless SPA on the app host | `""` |
| `t3.image.repository` | T3 SPA image repository | `aorwall/moatless-t3` |
| `t3.image.tag` | Image tag | `latest` |
| `t3.replicaCount` | Number of T3 SPA replicas | `1` |
| `t3.environmentId` | Stable id for this T3 environment | Release name |
| `t3.label` | Environment name shown in the client | `Moatless` |
| `t3.extraAllowedOrigins` | Extra origins allowed to open the backend WebSocket | `[]` |

`t3.extraAllowedOrigins` is only needed for a client served from another host —
a desktop or mobile build pointed at this deployment. The origin T3 is served
on is derived from `t3.host` (or `ingress.hosts[0].host`) and `proxy.protocol`,
so it is never configured twice.

### Kubernetes execution

| Parameter | Description | Default |
|-----------|-------------|---------|
| `kubernetes.enabled` | Enable Kubernetes execution mode | `true` |
| `kubernetes.namespace` | Namespace for sandbox pods | Release namespace |
| `kubernetes.sandboxStorage.enabled` | Enable per-task PVC | `true` |
| `kubernetes.sandboxStorage.defaultMode` | Default workspace tier (`ebs`/`ephemeral`/`s3`/`node`) | `s3` |
| `kubernetes.nodeArchive.enabled` | Track, retain, and clean node-local archives | `false` |
| `kubernetes.imagePrepull.enabled` | DaemonSet that pre-pulls the sandbox image | `true` |
| `kubernetes.nodeStorageJanitor.enabled` | Per-node cleanup of stale workspace/docker/task build-cache dirs when node archives are enabled | `true` |

#### Claude transcript persistence on S3

Each task's Claude config dir is regenerated on every boot except the conversation
transcripts (`~/.claude/projects/*.jsonl`), which Claude appends to across turns and
which `--resume` reads. The sandbox tars that directory and ships it to S3 **through
the backend** over the existing JWT-authed gRPC (sandbox pods are AWS-credential-less
by design), on a timer and on graceful shutdown, and restores it on the next start.

This persists transcripts off the per-task PVC or node disk and is the
off-node durability path for all storage tiers. In `node` mode the same
transcripts also live in the task hostPath. With node archive management
enabled, retained tasks are pinned back to their recorded node; `ephemeral`
relies on S3 for persistence.
It reuses the existing uploads bucket and the backend IRSA — no extra bucket,
IAM, or cluster infra. Objects land at
`s3://<uploads-bucket>/<prefix>/<task_id>/transcripts.tar` and are deleted on task
cleanup. Tune the upload cadence with `TRANSCRIPT_SYNC_INTERVAL_SECS` on the sandbox
(default 60s; `0` disables the timer but still flushes on shutdown).

> Scope the uploads bucket's lifecycle expiry to the `uploads/` prefix (or add a
> longer rule for `transcripts/`) so transcripts of long-lived tasks aren't deleted.

#### Enabling non-PVC storage tiers per repository

Storage tier is a per-repository setting stored in the repo's `resources` config.
Set `storageMode` to `ephemeral` to skip the per-task EBS PVC and run the workspace
on a node-local `emptyDir` (optionally cap its size with `storage`). New tasks for
that repo then provision with **no PVC**; transcripts persist only via the S3 sync
described above.

Set `storageMode` to `s3` to use the same node-local `emptyDir` plus git-tree
sync to S3. Set `storageMode` to `node` to use node-local hostPath storage under
`K8S_SANDBOX_CACHE_HOST_ROOT/workspaces/<task-id>`. This best-effort mode needs
no ConfigMap or node-reader permissions, but a replacement scheduled elsewhere
sees a fresh directory. The `node` tier has no Kubernetes
`emptyDir.sizeLimit`; it is bounded by node disk capacity.

Docker-in-Docker storage is controlled separately with `dockerStorageMode`.
For repos with `dockerAccess`, `dockerStorageMode: "node"` stores
`/var/lib/docker` on best-effort node-local hostPath storage so Docker layers,
images, and volumes can survive stop/start on the same node. Use
`dockerStorageMode: "emptyDir"` plus `dockerStorage` when you need a hard
Kubernetes size cap instead.

Build-cache storage is controlled with `buildCacheMode`. The default
`buildCacheMode: "emptyDir"` keeps `/opt/moatless/build-cache` as an emptyDir
capped by `buildCache` or `K8S_SANDBOX_BUILD_CACHE_SIZE`. Set
`buildCacheMode: "node"` to use hostPath storage under
`K8S_SANDBOX_CACHE_HOST_ROOT/build-cache`. `buildCacheScope: "task"` stores each
task under `build-cache/tasks/<task-id>` and is cleaned with the task; use
`buildCacheScope: "node"` to share one cache across all sandboxes scheduled on
the same Kubernetes node. HostPath build caches have no Kubernetes
`emptyDir.sizeLimit`; they are bounded by node disk capacity and cleanup policy.

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
    "storageMode": "s3",
    "storage": "30Gi",
    "buildCacheMode": "node",
    "buildCacheScope": "task",
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
moat repos update <repo-id> --storage-mode s3 --storage 30Gi --docker-storage-mode node --docker-storage 30Gi
```

Or via the API:

```bash
PATCH /api/.../repositories/<repo-id>   { "resources": { "storageMode": "s3", "dockerStorageMode": "node" } }
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

Then opt a repo's sandboxes into that identity with the per-repo
`sandboxServiceAccount` field (via `.moatless/config.json` or the repository API /
`moat repos create|update --sandbox-service-account moatless-dev`). Only that
repo's sandboxes run elevated. See `scripts/shadow/README.md` for the full flow.

> Dev clusters only. The field just *selects* the SA — RBAC (these bindings) is
> what grants power, so leave `shadowDev.enabled=false` in production.

### Ingress Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `ingress.enabled` | Emit the main Ingress object | `true` |
| `ingress.hosts[0].host` | App hostname (also read by the Traefik IngressRoutes) | `vibe.example.com` |
| `proxy.traefikIngressRoute.enabled` | Emit the app's Traefik IngressRoute CRDs | `false` |
| `proxyIngress.enabled` | Emit the wildcard preview Ingress | `true` |
| `proxyIngress.host` | Wildcard host for previews | `*.preview.example.com` |

> The Traefik IngressRoutes read the app host from `ingress.hosts[0].host`
> regardless of `ingress.enabled`. Keep `ingress.hosts[0].host` set even when you
> disable the Ingress object, or the routes fall back to `vibe.example.com`.

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
