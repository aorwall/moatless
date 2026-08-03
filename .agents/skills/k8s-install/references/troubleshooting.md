# Troubleshooting

Failure modes grouped by step. Diagnose with `kubectl -n <ns> …`; most can be fixed without re-running the whole install.

## Preflight

- **`kubectl context` missing** → `kubectl config use-context <name>`; run `aws eks update-kubeconfig` or `az aks get-credentials` first if the context isn't even set.
- **`cluster reachable` false** → credentials expired. For AKS: `az aks get-credentials --name <cl> --resource-group <rg>`. For EKS: `aws eks update-kubeconfig --name <cl>`.

## Init

- **`values.local.yaml` wrong domain/auth** → re-run `moat k8s init` with `--force` to overwrite, or edit the file directly.

## Secrets

- **`kubectl apply` denied** → namespace RBAC. User needs `secrets` create/patch in the namespace.
- **`.env` values ignored** → file path must be at repo root; use `--env-file <path>` for other locations.

## Install

- **`rendered manifests contain a resource that already exists`** → prior broken install. Run `helm uninstall moatless-vibe -n <ns>` first, or pass a new release name.
- **`no matches for kind "IngressRoute"`** → Traefik CRDs are missing. Install Traefik (or its CRDs) before the chart; the app IngressRoutes need them. See `references/prerequisites.md`.
- **Pods crash-looping on startup** → `kubectl -n <ns> logs deploy/moatless-vibe-backend --previous`. Common: bad `DATABASE_URL`, migration failure.

## Image pulls

- **`ImagePullBackOff`** → `ghcr-secret` missing in the namespace. `moat k8s secrets pull-token` and then `kubectl -n <ns> rollout restart deploy`.
- **`403 Forbidden` on GHCR** → PAT lacks `read:packages` scope. Recreate the token.

## Storage

- **`PVC pending`** → `kubectl -n <ns> describe pvc <name>`. Either no default StorageClass (fix class annotation) or zone mismatch.

## Ingress

- **Ingress address `<pending>` forever** (AKS) → Web app routing not enabled. `az aks approuting enable`.
- **Ingress address `<pending>` forever** (EKS) → AWS Load Balancer Controller not installed or missing IAM. `kubectl -n kube-system logs deploy/aws-load-balancer-controller`.
- **DNS not resolving** → need to create a CNAME/A record pointing at the ingress address after install. `kubectl -n <ns> get ingress` shows the target.
- **502 from ALB** → healthcheck path wrong. Add `alb.ingress.kubernetes.io/healthcheck-path: /health` to ingress annotations.

## Verify

- **Backend pod `Running` but status fails** → migrations crashed. `kubectl -n <ns> logs deploy/moatless-vibe-backend | grep -i migra`.
- **Sandbox pods stuck `ContainerCreating`** → network policy blocks the init container. Inspect `kubectl -n <ns> get networkpolicy` and see the Cilium notes in `provider-*.md`.
- **Backend warns `cannot ... resource "networkpolicies"`** → release predates the pod-manager Role's `networking.k8s.io/networkpolicies` grant. Preview NetworkPolicies are skipped until `helm upgrade` reapplies RBAC; verify with `kubectl -n <ns> auth can-i delete networkpolicies --as=system:serviceaccount:<ns>:<backend-sa>`.
