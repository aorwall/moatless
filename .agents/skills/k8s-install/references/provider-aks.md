# AKS Provider Notes

## Detection

`moat k8s preflight` reports `provider: aks` when node `/spec/providerID` starts with `azure://`.

## Ingress — Web App Routing

Preferred AKS ingress. Enable once:

```bash
az aks approuting enable --name <cluster> --resource-group <rg>
```

This installs an nginx-backed controller exposing the `webapprouting.kubernetes.azure.com` IngressClass. Set in `values.local.yaml`:

```yaml
ingress:
  className: webapprouting.kubernetes.azure.com
proxyIngress:
  className: webapprouting.kubernetes.azure.com
```

## DNS

AKS web app routing uses Azure DNS zones. Either:
- Bring your own DNS and point A records at the ingress `LoadBalancer` external IP (`kubectl -n app-routing-system get svc`)
- Or attach a managed Azure DNS zone: `az aks approuting zone add --ids <zone-resource-id>`

## Storage

Default StorageClass `managed-csi` is present out of the box — keep `kubernetes.sandboxStorage.storageClass` empty.

For better IO on sandbox workloads use `managed-csi-premium` (SSD). Patch via values:

```yaml
kubernetes:
  sandboxStorage:
    storageClass: managed-csi-premium
```

## TLS

Two common setups:

1. **Azure Front Door in front** — terminate TLS at Front Door, leave `ingress.tls: []`. Traffic to the cluster stays HTTP over private link.
2. **cert-manager in cluster** — install cert-manager, add a ClusterIssuer, populate `ingress.tls` in values.

The installer leaves `tls: []` by default; come back and edit after verifying the base install.

## Monitoring

If the cluster has Azure Monitor for Containers enabled, `monitoring.prometheus.enabled=false` is fine — Azure scrapes logs/metrics automatically. Only turn on `monitoring.prometheus.enabled=true` (PodMonitor scraping) if you also install prometheus-operator yourself. Grafana dashboards (`monitoring.grafana.enabled`) are a separate, independent switch.

## Sysctls

Sandbox pods set no unusual sysctls. No DaemonSet required on AKS.

## Common AKS issues

| Symptom | Cause | Fix |
|---|---|---|
| Ingress address stays `<pending>` | Web app routing not enabled | `az aks approuting enable …` |
| `ImagePullBackOff` on backend | ghcr-secret missing in namespace | `moat k8s secrets pull-token` |
| PVC stuck `Pending` | Zone mismatch — AKS PVs are zonal | Pin deployment to a single zone or use `managed-csi` (zone-agnostic) |
