# Moatless Vibe Helm Chart

A Helm chart for deploying Moatless Vibe - an AI-powered development platform.

## Prerequisites

- Kubernetes 1.19+
- Helm 3.0+
- Redis 7.0+ (required for event streaming)
- PostgreSQL 14+ (for production deployments)

## Redis Configuration

**Redis is required** for event streaming between the main application and sandbox pods. You have three deployment options:

### Option 1: Deploy Redis with Chart (Development/Testing)

The chart includes an optional Redis deployment using the Bitnami Redis subchart. This is suitable for development and testing environments.

```bash
# Install with bundled Redis
helm install moatless ./k8s/helm/moatless-vibe \
  --set redis.enabled=true \
  --set secrets.create=true \
  --set secrets.anthropicApiKey="your-api-key" \
  --set secrets.githubToken="your-github-token"
```

**Configuration:**
```yaml
redis:
  enabled: true
  auth:
    enabled: false  # For development only
  master:
    persistence:
      enabled: true
      size: 1Gi
  replica:
    replicaCount: 1
```

**⚠️ Warning:** The bundled Redis deployment is NOT recommended for production. Use a managed Redis service instead.

### Option 2: Use Managed Redis (Production - Recommended)

For production deployments, use a managed Redis service from your cloud provider:

#### AWS ElastiCache for Redis

```bash
# 1. Create ElastiCache cluster (Terraform example)
resource "aws_elasticache_replication_group" "redis" {
  replication_group_id       = "moatless-redis"
  description                = "Redis for Moatless Vibe event streaming"
  engine                     = "redis"
  engine_version             = "7.0"
  node_type                  = "cache.t4g.small"
  number_cache_clusters      = 2
  automatic_failover_enabled = true
  multi_az_enabled           = true
  at_rest_encryption_enabled = true
  transit_encryption_enabled = false  # Set to true if using AUTH
}

# 2. Install chart with ElastiCache endpoint
helm install moatless ./k8s/helm/moatless-vibe \
  --set redis.url="redis://moatless-redis.abc123.use1.cache.amazonaws.com:6379" \
  --set secrets.create=true \
  --set secrets.anthropicApiKey="your-api-key" \
  --set secrets.githubToken="your-github-token" \
  --set secrets.databaseUrl="postgresql://user:pass@host:5432/db"
```

#### Azure Cache for Redis

```bash
# With authentication
helm install moatless ./k8s/helm/moatless-vibe \
  --set redis.url="redis://:password@my-cache.redis.cache.windows.net:6380" \
  --set secrets.create=true \
  --set secrets.anthropicApiKey="your-api-key"
```

#### Google Cloud Memorystore

```bash
helm install moatless ./k8s/helm/moatless-vibe \
  --set redis.url="redis://10.0.0.3:6379" \
  --set secrets.create=true \
  --set secrets.anthropicApiKey="your-api-key"
```

### Option 3: Use Existing Redis Cluster

If you already have a Redis deployment in your cluster:

```bash
helm install moatless ./k8s/helm/moatless-vibe \
  --set redis.url="redis://my-redis-service.default.svc.cluster.local:6379" \
  --set secrets.create=true \
  --set secrets.anthropicApiKey="your-api-key"
```

## Installation

### Quick Start (Development)

```bash
# 1. Update Helm dependencies (required for Redis subchart)
helm dependency update ./k8s/helm/moatless-vibe

# 2. Install with bundled Redis
helm install moatless ./k8s/helm/moatless-vibe \
  --set redis.enabled=true \
  --set secrets.create=true \
  --set secrets.anthropicApiKey="sk-ant-..." \
  --set secrets.githubToken="ghp_..." \
  --set ingress.hosts[0].host=vibe.local \
  --set proxyIngress.host="*.preview.local"
```

### Production Installation

```bash
# 1. Create secrets (recommended: use external secrets manager)
kubectl create secret generic vibe-secrets \
  --from-literal=ANTHROPIC_API_KEY="sk-ant-..." \
  --from-literal=GITHUB_TOKEN="ghp_..." \
  --from-literal=SANDBOX_API_KEY="$(openssl rand -base64 32)" \
  --from-literal=DATABASE_URL="postgresql://user:pass@host:5432/db"

# 2. Update Helm dependencies
helm dependency update ./k8s/helm/moatless-vibe

# 3. Install with production values
helm install moatless ./k8s/helm/moatless-vibe \
  --set redis.url="redis://your-redis-endpoint:6379" \
  --set externalSecrets.enabled=true \
  --set externalSecrets.secretName=vibe-secrets \
  --set ingress.hosts[0].host=vibe.example.com \
  --set proxyIngress.host="*.preview.example.com"
```

## Configuration

### Redis Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `redis.enabled` | Deploy Redis with chart | `false` |
| `redis.url` | External Redis connection URL | `""` |
| `redis.auth.enabled` | Enable Redis authentication | `false` |
| `redis.master.persistence.enabled` | Enable persistence for master | `true` |
| `redis.master.persistence.size` | Storage size for master | `1Gi` |
| `redis.replica.replicaCount` | Number of Redis replicas | `1` |

### Application Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `replicaCount` | Number of API replicas | `1` |
| `image.repository` | API image repository | `aorwall/moatless-frontend` |
| `image.tag` | Image tag | Chart `appVersion` |
| `sandboxImage.repository` | Sandbox image repository | `aorwall/moatless-sandbox` |

### Database Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `database.provider` | Database provider (sqlite or postgres) | `postgres` |
| `database.urlSecretKey` | Secret key for DATABASE_URL | `DATABASE_URL` |

### Kubernetes Execution

| Parameter | Description | Default |
|-----------|-------------|---------|
| `kubernetes.enabled` | Enable Kubernetes execution mode | `true` |
| `kubernetes.namespace` | Namespace for sandbox pods | Release namespace |
| `kubernetes.sandboxStorage.enabled` | Enable per-task PVC | `true` |
| `kubernetes.sandboxStorage.size` | Default workspace size for per-task PVC and emptyDir tiers (per-repo override via `.moatless/config.json` → `resources.storage`) | `3Gi` |
| `kubernetes.sandboxCacheHostRoot` | Host root for node-local sandbox caches and best-effort workspace/docker storage | `/mnt/cache` |
| `kubernetes.nodeStorageJanitor.enabled` | Enable per-node cleanup for stale node-local workspace/docker directories | `true` |
| `kubernetes.nodeStorageJanitor.retentionHours` | Age threshold before per-task node-local directories are pruned | `168` |
| `kubernetes.transcriptStorage.prefix` | S3 key prefix for synced Claude transcripts (`<prefix>/<task_id>/transcripts.tar`) | `transcripts` |

#### Claude transcript persistence on S3

Each task's Claude config dir is regenerated on every boot except the conversation
transcripts (`~/.claude/projects/*.jsonl`), which Claude appends to across turns and
which `--resume` reads. The sandbox tars that directory and ships it to S3 **through
the backend** over the existing JWT-authed gRPC (sandbox pods are AWS-credential-less
by design), on a timer and on graceful shutdown, and restores it on the next start.

This persists transcripts off the per-task PVC and is the **only** guaranteed
transcript durability for non-EBS storage tiers (`ephemeral`, `s3`, and `node`).
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
sync to S3. Set `storageMode` to `node` to use best-effort node-local hostPath
storage under `K8S_SANDBOX_CACHE_HOST_ROOT/workspaces/<task-id>`: stop/start on
the same node reuses the workspace, while scheduling on another node starts with
an empty directory. The `node` tier has no Kubernetes `emptyDir.sizeLimit`; it is
bounded by node disk capacity.

Docker-in-Docker storage is controlled separately with `dockerStorageMode`.
For repos with `dockerAccess`, `dockerStorageMode: "node"` stores
`/var/lib/docker` on best-effort node-local hostPath storage so Docker layers,
images, and volumes can survive stop/start on the same node. Use
`dockerStorageMode: "emptyDir"` plus `dockerStorage` when you need a hard
Kubernetes size cap instead.

Kubernetes does not delete hostPath contents when sandbox pods are removed. The
chart enables `kubernetes.nodeStorageJanitor` by default, which runs on sandbox
nodes and prunes stale per-task directories under
`kubernetes.sandboxCacheHostRoot/workspaces` and
`kubernetes.sandboxCacheHostRoot/docker` after the task's Deployment is gone and
the directory is older than `retentionHours` (default 168). This bounds node disk
growth while preserving best-effort same-node reuse for recently stopped tasks.

Declaratively via `.moatless/config.json`, then sync:

```json
{
  "resources": {
    "storageMode": "s3",
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
moat repos update <repo-id> --storage-mode s3 --storage 30Gi --docker-storage-mode node --docker-storage 30Gi
```

Or via the API:

```bash
PATCH /api/.../repositories/<repo-id>   { "resources": { "storageMode": "s3", "dockerStorageMode": "node" } }
```

### Ingress Parameters

| Parameter | Description | Default |
|-----------|-------------|---------|
| `ingress.enabled` | Enable main ingress | `true` |
| `ingress.className` | Ingress class name | `""` |
| `ingress.hosts[0].host` | API hostname | `vibe.example.com` |
| `proxyIngress.enabled` | Enable proxy ingress | `true` |
| `proxyIngress.host` | Wildcard host for previews | `*.preview.example.com` |

## Redis Requirements

### Production Checklist

- ✅ **Version**: Redis 7.0 or higher
- ✅ **Persistence**: AOF or RDB snapshots enabled
- ✅ **High Availability**: Master + 1-2 replicas recommended
- ✅ **Memory**: 2-4GB recommended (varies with workload)
- ✅ **Network**: Low latency to Kubernetes cluster
- ✅ **Security**: Use authentication in production (AUTH command)
- ✅ **Monitoring**: Track memory usage, connection counts, stream length

### Redis Streams Configuration

The application uses Redis Streams for event delivery with the following settings:

- **Stream naming**: `task-events:{taskId}` (one stream per task)
- **Consumer group**: `main-app-consumers` (configurable via `REDIS_CONSUMER_GROUP`)
- **Stream trimming**: ~10,000 messages per stream (configurable via `REDIS_MAX_STREAM_LENGTH`)
- **Blocking reads**: 5-second timeout (configurable via `REDIS_BLOCK_MS`)

## Monitoring

### Redis Monitoring

If using managed Redis:
- **AWS CloudWatch**: Monitor ElastiCache metrics (CPUUtilization, DatabaseMemoryUsagePercentage, NetworkBytesIn/Out)
- **Azure Monitor**: Track Cache for Redis metrics (Server Load, Memory Usage, Connected Clients)
- **Prometheus**: Use Redis exporters for detailed metrics

Application-level metrics are exposed at `/metrics` endpoint.

## Troubleshooting

### Redis Connection Issues

**Symptom**: Application logs show "Failed to connect to Redis"

**Solutions**:
1. Verify Redis URL is correct:
   ```bash
   kubectl get configmap moatless-vibe-config -o yaml | grep REDIS_URL
   ```

2. Test connectivity from a pod:
   ```bash
   kubectl run redis-test --rm -it --image=redis:7-alpine -- redis-cli -u "redis://your-redis:6379" PING
   ```

3. Check network policies and security groups

### Event Delivery Issues

**Symptom**: Sandbox events not appearing in UI

**Solutions**:
1. Check Redis streams exist:
   ```bash
   redis-cli KEYS "task-events:*"
   redis-cli XINFO GROUPS task-events:{taskId}
   ```

2. Verify consumer group health:
   ```bash
   redis-cli XPENDING task-events:{taskId} main-app-consumers
   ```

3. Check application logs:
   ```bash
   kubectl logs -l app.kubernetes.io/name=moatless-vibe --tail=100
   ```

### Redis Out of Memory

**Symptom**: Redis evicting keys or refusing writes

**Solutions**:
1. Check stream lengths:
   ```bash
   redis-cli XLEN task-events:{taskId}
   ```

2. Increase `REDIS_MAX_STREAM_LENGTH` (reduces memory usage):
   ```bash
   helm upgrade moatless ./k8s/helm/moatless-vibe \
     --reuse-values \
     --set extraEnv.REDIS_MAX_STREAM_LENGTH=5000
   ```

3. Increase Redis memory limit (managed service) or node type

## Upgrading

```bash
# Update dependencies
helm dependency update ./k8s/helm/moatless-vibe

# Upgrade release
helm upgrade moatless ./k8s/helm/moatless-vibe \
  --reuse-values \
  --set image.tag=v0.2.0
```

## Uninstalling

```bash
# Uninstall release
helm uninstall moatless

# Clean up Redis PVCs (if bundled Redis was enabled)
kubectl delete pvc -l app.kubernetes.io/name=redis
```

## Examples

### Development Setup (Local Kubernetes)

```bash
# 1. Create local cluster (kind/k3d/minikube)
kind create cluster --name moatless

# 2. Install with bundled Redis
helm dependency update ./k8s/helm/moatless-vibe
helm install moatless ./k8s/helm/moatless-vibe \
  --set redis.enabled=true \
  --set secrets.create=true \
  --set secrets.anthropicApiKey="sk-ant-..." \
  --set secrets.githubToken="ghp_..." \
  --set ingress.enabled=false \
  --set service.type=LoadBalancer

# 3. Access application
kubectl port-forward svc/moatless-moatless-vibe 8000:8000
```

### AWS EKS Production Setup

```bash
# 1. Create ElastiCache cluster (via Terraform/CloudFormation)
# 2. Create RDS PostgreSQL instance
# 3. Create secrets
kubectl create secret generic vibe-secrets \
  --from-literal=ANTHROPIC_API_KEY="sk-ant-..." \
  --from-literal=GITHUB_TOKEN="ghp_..." \
  --from-literal=SANDBOX_API_KEY="$(openssl rand -base64 32)" \
  --from-literal=DATABASE_URL="postgresql://..."

# 4. Install chart
helm dependency update ./k8s/helm/moatless-vibe
helm install moatless ./k8s/helm/moatless-vibe \
  --set redis.url="redis://moatless-redis.abc123.use1.cache.amazonaws.com:6379" \
  --set externalSecrets.enabled=true \
  --set externalSecrets.secretName=vibe-secrets \
  --set ingress.enabled=true \
  --set ingress.className=alb \
  --set ingress.annotations."alb\.ingress\.kubernetes\.io/scheme"=internet-facing \
  --set ingress.annotations."alb\.ingress\.kubernetes\.io/target-type"=ip \
  --set ingress.hosts[0].host=vibe.example.com \
  --set proxyIngress.host="*.preview.example.com"
```

## Support

For issues and questions:
- GitHub Issues: https://github.com/aorwall/moatless-vibe/issues
- Documentation: https://github.com/aorwall/moatless-vibe/tree/main/docs
