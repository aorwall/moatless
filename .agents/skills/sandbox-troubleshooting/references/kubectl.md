# Kubernetes lookup and permissions

Read `sandboxId` from `moat tasks get <task-id> --json`. Use the target context
and configured sandbox namespace in every command:

```bash
kubectl --context <context> -n <namespace> get pods -l moatless.io/sandbox-id=<sandbox-id> -o wide
kubectl --context <context> -n <namespace> describe pod <pod>
kubectl --context <context> -n <namespace> get events --field-selector involvedObject.name=<pod> --sort-by=.lastTimestamp
kubectl --context <context> -n <namespace> logs <pod> -c <container> --tail=500
kubectl --context <context> -n <namespace> logs <pod> -c <container> --previous --tail=500
kubectl --context <context> -n <namespace> top pod <pod> --containers
```

Select the pod by UID, creation time, and container state if more than one matches.
Do not filter only Running pods: Pending, failed, and evicted pods contain the
cause of provisioning failures. Running also does not imply all containers are ready.
Names and labels come from [pod_builder.rs](../../../../crates/sandbox-k8s/src/pod_builder.rs)
and [naming.rs](../../../../crates/sandbox-k8s/src/naming.rs).

Discover the actual pod service account and test its permissions:

```bash
kubectl --context <context> -n <namespace> get pod <pod> -o jsonpath='{.spec.serviceAccountName}'
kubectl --context <context> -n <namespace> auth can-i get pods
kubectl --context <context> -n <namespace> auth can-i list events
kubectl --context <context> -n <namespace> auth can-i get pods.metrics.k8s.io
```

These commands test the current caller, not automatically the pod account. An
impersonated check needs permission to impersonate. Missing metrics can mean
missing RBAC or metrics-server; it does not mean zero resource use.
