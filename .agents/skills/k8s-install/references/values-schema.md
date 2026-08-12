# values.local.yaml — Schema Cheat Sheet

The chart at `k8s/helm/moatless-vibe/` exposes ~100 values; `moat k8s init` populates the minimum required set. This doc explains which to leave alone and which to review before `install`.

## Image

```yaml
image:
  repository: ghcr.io/aorwall/moatless-vibe-backend
  tag: <version>   # set by --tag flag
sandboxImage:
  repository: ghcr.io/aorwall/moatless-sandbox
  tag: <version>   # typically same as image.tag
```

Override `image.tag` / `sandboxImage.tag` to pin versions. `moat k8s upgrade --tag <v>` updates both in one go.

## Ingress (main app)

```yaml
ingress:
  enabled: true
  className: <provider-specific>  # webapprouting.kubernetes.azure.com | alb | nginx | traefik
  annotations: { ... }             # provider-specific, see provider-*.md
  hosts:
    - host: app.example.com
      paths:
        - path: /
          pathType: Prefix
  tls: []                          # populate if terminating TLS in-cluster
```

## Proxy ingress (sandbox previews)

```yaml
proxyIngress:
  enabled: true
  className: <same as ingress>
  host: "*.sandbox.example.com"
  tls:
    secretName: proxy-wildcard-tls
    hosts: ["*.sandbox.example.com"]
```

Preview URLs take the shape `<taskId>--<port>.<proxyDomain>`, so the proxy must terminate a wildcard cert.

## Auth

```yaml
auth:
  provider: local | github_app | auth0
  frontendUrl: https://<domain>
  githubApp:
    enabled: true
    callbackUrl: https://<domain>/api/v1/auth/github-app/callback
    appId: "<github-app-id>"
  auth0:
    enabled: true
    callbackUrl: https://<domain>/api/auth/auth0/callback
```

Set exactly one provider. Credentials (client ID/secret) go in the Secret, not values.

## Data stores

```yaml
# Production: point at managed data stores via the Secret.
postgres:
  enabled: false   # bundled Postgres off → supply DATABASE_URL in the Secret
redis:
  enabled: false   # bundled Redis off → supply redis.url (or REDIS_URL in the Secret)
  url: "redis://your-redis:6379"
```

For production, keep both bundled data stores off and supply `DATABASE_URL` via `moat k8s secrets bootstrap` and an external `redis.url`. The bundled first-party Postgres (`postgres.enabled: true`) and Redis (`redis.enabled: true`, the default) are single-node and meant for local/demo installs, not production.

## Storage

```yaml
kubernetes:
  sandboxStorage:
    storageClass: ""  # empty string → use default StorageClass
    size: 20Gi        # per-sandbox PVC
```

## Secrets (important)

```yaml
secrets:
  create: false        # moat k8s secrets bootstrap owns this
externalSecrets:
  enabled: true        # tells chart the Secret exists out-of-band
```

`moat k8s init` always sets these — keep that way. The chart won't try to template a Secret from chart values (which would leak them into `values.yaml`).

## Pull secret

```yaml
imagePullSecrets:
  - name: ghcr-secret
```

Created by `moat k8s secrets pull-token` when pulling from GHCR.

## Resource sizing

```yaml
resources:
  requests: { cpu: 200m, memory: 512Mi }
  limits:   { cpu: 2,    memory: 2Gi }
```

Safe defaults. Tune up for heavy users or add an HPA via `autoscaling.enabled`.

## Monitoring (optional)

```yaml
monitoring:
  prometheus:
    enabled: false          # PodMonitor scraping; requires prometheus-operator CRDs
    scrapeInterval: "30s"
    apiVersion: "monitoring.coreos.com/v1"
  grafana:
    enabled: false          # bundled dashboards as a ConfigMap; no CRD needed
    dashboardLabels: {}
```

`prometheus.enabled` needs prometheus-operator installed; `grafana.enabled` is an independent switch that just ships a dashboard ConfigMap. Leave both off unless you use them.
