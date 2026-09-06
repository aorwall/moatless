# Installation failure diagnosis

```bash
moat k8s status --context <context> --namespace <namespace> --json
kubectl --context <context> -n <namespace> get pods,services,ingress
kubectl --context <context> -n <namespace> get events --sort-by=.lastTimestamp
kubectl --context <context> -n <namespace> logs <pod> -c <container> --tail=500
```

| Failure | Investigate |
|---|---|
| Pending | Scheduling events, storage binding, quotas, node/compute constraints. |
| ImagePullBackOff | Exact image tag, registry credentials, supported architecture. |
| Backend boot fails | Logs, app Secret names, database access, encryption-key shape, migrations. |
| App works but preview fails | Wildcard DNS/TLS, proxy Ingress to Traefik, CRDs, sandbox namespace routing and auth middleware. |
| Login callback fails | Public URL, provider callback including `/api/v1`, cookie/TLS configuration. |
| Sandbox denied by API | Actual service account and its namespace-scoped permissions. |

Use [sandbox-troubleshooting](../../sandbox-troubleshooting/SKILL.md) after the
installation is running. Capture diagnostics before replacing a failed pod;
never wipe PVCs or rotate the master encryption key as a generic retry.
