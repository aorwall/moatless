# Prerequisites — Install Snippets

Moatless Vibe relies on several cluster-level pieces. The skill detects what is missing in preflight; this page is the remediation library.

## Traefik (for IngressRoute CRDs)

Sandbox preview URLs rely on Traefik `IngressRoute` CRDs for per-task routing. Only required if `proxyIngress.backend == traefik` (the default for some older values files).

```bash
helm repo add traefik https://traefik.github.io/charts
helm repo update
helm install traefik traefik/traefik \
  --namespace traefik --create-namespace \
  --set ingressRoute.dashboard.enabled=false
```

CRD verification:
```bash
kubectl get crd ingressroutes.traefik.io
```

Alternative: if the cluster uses a cloud-native ingress (AKS web app routing, ALB), set `proxyIngress.className` to that class in values and skip Traefik entirely — the chart generates an Ingress instead of an IngressRoute when the class is not Traefik.

## cert-manager (for in-cluster TLS)

Skip when TLS terminates upstream (Azure Front Door, ALB with ACM cert).

```bash
helm repo add jetstack https://charts.jetstack.io
helm repo update
helm install cert-manager jetstack/cert-manager \
  --namespace cert-manager --create-namespace \
  --set crds.enabled=true
```

ClusterIssuer for Let's Encrypt:
```yaml
apiVersion: cert-manager.io/v1
kind: ClusterIssuer
metadata:
  name: letsencrypt-prod
spec:
  acme:
    server: https://acme-v02.api.letsencrypt.org/directory
    email: admin@example.com
    privateKeySecretRef:
      name: letsencrypt-prod
    solvers:
      - http01:
          ingress:
            class: <ingress-class>
```

## External Postgres

Moatless expects Postgres 14+. Providers:

- **Azure Database for PostgreSQL Flexible Server** — pair with AKS, use private endpoint in the AKS vnet
- **AWS RDS Postgres** — enable private access, security group open to EKS node SG on 5432
- **Supabase / Neon / Managed DB** — works but latency matters; keep region-local

Set `DATABASE_URL` in the Secret: `postgres://user:pass@host:5432/vibe?sslmode=require`.

## External Redis

Required for event streams. Small (`cache.t3.micro` / `Standard_B1ms`) is fine.

Set `REDIS_URL` in the Secret: `redis://:<password>@host:6379/0` (use `rediss://` for TLS).

## Prometheus (optional)

Only if `monitoring.prometheus.enabled=true` in values (PodMonitor scraping). Install kube-prometheus-stack:

```bash
helm repo add prometheus-community https://prometheus-community.github.io/helm-charts
helm install kube-prom prometheus-community/kube-prometheus-stack \
  --namespace monitoring --create-namespace
```

Provides `ServiceMonitor` CRD the chart needs.
