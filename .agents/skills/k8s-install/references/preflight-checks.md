# Preflight Checks — What Each Means

`moat k8s preflight` is a read-only audit. Every check it emits falls into one of these categories.

## Tooling

| Check | Means | Fix |
|---|---|---|
| `kubectl installed` | `kubectl` binary is on PATH | Install kubectl: https://kubernetes.io/docs/tasks/tools/ |
| `helm installed` | `helm` 3.x binary is on PATH | Install helm: https://helm.sh/docs/intro/install/ |

Hard requirement — everything else needs these two.

## Cluster connectivity

| Check | Means | Fix |
|---|---|---|
| `kubectl context` | A current context is selected | `kubectl config use-context <name>` |
| `cluster reachable` | `kubectl get nodes` returns | Verify credentials; `kubectl cluster-info` should succeed |

Provider detection runs here. Nodes' `/spec/providerID` tells the CLI whether the cluster is AKS (`azure://...`), EKS (`aws:///...`), or something else (reported as `generic`).

## Namespace

| Check | Means | Fix |
|---|---|---|
| `namespace '<ns>' exists` | The target namespace is already present | Create via `kubectl create namespace <ns>` or let helm's `--create-namespace` handle it on install |

Not hard-fatal; helm will create the namespace at install time.

## Ingress

| Check | Means | Fix |
|---|---|---|
| `ingress controller present` | A Deployment with `app.kubernetes.io/name in (ingress-nginx, traefik, aws-load-balancer-controller)` exists (or an IngressClass points to a cloud controller) | See provider-aks.md / provider-eks.md |

Hard requirement. If missing, external traffic cannot reach the app.

## Storage

| Check | Means | Fix |
|---|---|---|
| `default StorageClass present` | At least one StorageClass has the `storageclass.kubernetes.io/is-default-class=true` annotation | AKS: `managed-csi` is typically default; EKS: install EBS CSI driver (`eksctl create addon --name aws-ebs-csi-driver`) |

Can be worked around by setting `kubernetes.sandboxStorage.storageClass` explicitly in `values.local.yaml`, but a default simplifies everything.

## Custom Resource Definitions

| CRD Check | When needed |
|---|---|
| `traefikroutes CRD` | If using Traefik for per-sandbox proxy routes. Moatless uses `IngressRoute` CRDs to carve out preview URLs under the proxy domain. |
| `cert-manager CRD` | If terminating TLS inside the cluster. Skip if upstream (AKS web app routing, ALB) handles certs. |
| `prometheus-operator CRD` | Only if enabling PodMonitor scraping (`monitoring.prometheus.enabled=true` in values). Bundled Grafana dashboards (`monitoring.grafana.enabled=true`) are a plain ConfigMap and don't need this CRD. |
| `cilium CRD` | Only if you want Cilium network policies; skip otherwise. |

CRDs are soft requirements — install them only when the matching feature is turned on.

## Interpreting report.ok

`report.all_healthy` is `true` only when every `check.ok == true`. That's the green light for Step 2. Everything else should be surfaced to the user with its `hint` before proceeding.
