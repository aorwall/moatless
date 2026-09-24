# AKS

Use the actual target context and inspect its controllers and storage classes:

```bash
kubectl --context <context> get ingressclass
kubectl --context <context> get storageclass
kubectl --context <context> -n <namespace> get services
```

The [AKS values example](../examples/values-aks-minimal.yaml) targets an existing
`webapprouting.kubernetes.azure.com` Ingress setup. Confirm its class and TLS
secrets before applying it. Keep Traefik for the preview hop described in
[prerequisites](prerequisites.md).

Microsoft is retiring the NGINX application-routing add-on. Check the [current Azure routing guidance](https://learn.microsoft.com/en-us/azure/aks/app-routing-gateway-api)
before a fresh install. The chart emits Ingress resources; adopting Gateway API
requires a separate routing change, not just a different class name in this example.

Use the cluster's verified CSI StorageClass for PVC-backed sandboxes. Confirm
Postgres reachability from the workload network and use the appropriate
Azure workload identity or explicitly scoped object-store credentials.
