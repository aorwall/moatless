# Preflight interpretation

```bash
moat k8s preflight --context <context> --namespace <namespace> --json
kubectl --context <context> cluster-info
kubectl --context <context> get ingressclass
kubectl --context <context> get storageclass
kubectl --context <context> get crd ingressroutes.traefik.io
kubectl --context <context> -n <namespace> get service traefik
```

Use the configured Traefik service name if it differs. Preflight reports checks
and hints, not proof of end-to-end preview routing. Confirm the sandbox namespace
and permissions separately, including cross-namespace routing when applicable.

The installer `--storage-mode` enum is a preflight/infrastructure setting. It is
not a workspace ResourceConfig field: workspace overrides use `pvcEnabled` and
`s3Enabled`. Fargate requires its matching compute/storage preflight and validated
infrastructure handoff. Do not provision an EBS dependency for Fargate pods.
