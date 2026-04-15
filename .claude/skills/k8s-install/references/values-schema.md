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
  provider: "" | github | auth0
  frontendUrl: https://<domain>
  github:
    enabled: true
    callbackUrl: https://<domain>/api/auth/github/callback
  auth0:
    enabled: true
    callbackUrl: https://<domain>/api/auth/auth0/callback
```

Set exactly one provider. Credentials (client ID/secret) go in the Secret, not values.

## Data stores

```yaml
database:
  external: true   # use DATABASE_URL from Secret
redis:
  external: true   # use REDIS_URL from Secret
```

Leave `external: true` and supply URLs via `moat k8s secret bootstrap`. Bundled Postgres/Redis charts exist but are meant for demos, not production.

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
  create: false        # moat k8s secret bootstrap owns this
externalSecrets:
  enabled: true        # tells chart the Secret exists out-of-band
```

`moat k8s init` always sets these — keep that way. The chart won't try to template a Secret from chart values (which would leak them into `values.yaml`).

## Pull secret

```yaml
imagePullSecrets:
  - name: ghcr-secret
```

Created by `moat k8s secret pull-token` when pulling from GHCR.

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
  enabled: false   # requires prometheus-operator CRDs
```

Leave off unless the cluster has prometheus-operator installed.
