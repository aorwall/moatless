# kubectl from inside a sandbox — RBAC, pod-name derivation, jsonpath

## What SA are you?

Sandbox pods on this cluster run as `system:serviceaccount:moatless:moatless-sandbox`
(a dedicated identity for EKS Pod Identity → CodeArtifact, set via
`kubernetes.sandboxServiceAccount` in the gitops values). Check with:

```bash
kubectl auth whoami
kubectl auth can-i --list -n moatless
```

## Tested capability matrix (moatless-sandbox SA)

| Command | Works? | Notes |
|---|---|---|
| `kubectl get pods -o wide` | ✅ | STATUS, RESTARTS, NODE, IP |
| `kubectl get deploy/rs/statefulset` | ✅ | replicas, ready |
| `kubectl logs [-c sandbox] [--previous]` | ✅ | current + crashed instance |
| `kubectl describe pod` | ✅ | Events section populates (events granted, see below) |
| `kubectl get events` / describe events | ✅ | via the `-observability` RoleBinding (chart ≥ 0.1.1) |
| `kubectl get nodes` / `kubectl top nodes` | ✅ | node CPU/mem — for node-pressure checks |
| `kubectl top pods` / pods.metrics | ✅ | via the `-observability` RoleBinding (needs metrics-server) |
| `kubectl exec` / `kubectl port-forward` | ❌ | **Forbidden** — SA can't `create pods/exec` or `pods/portforward` (needs admin/backend SA) |
| `kubectl scale deploy` (patch) | ✅ | patch allowed; **no delete/create** |
| `kubectl get secret <name>` | ✅ | get only, not list |
| `kubectl get pvc` | ❌ | Forbidden |
| `kubectl get role/rolebinding` | ❌ | Forbidden |
| deploy delete/create | ❌ | patch-only |

`kubectl get events` and `kubectl top pods` are granted by the `-observability`
Role once the sandbox ServiceAccount is bound to it (see "Enabling …" below).
If you're on an **older release** where that binding isn't present yet, both
return `Forbidden` — fall back to the `moat` CLI (`moat tasks diagnose` /
`moat events list`, which the backend serves with its own permissions) or the
pod-JSON jsonpath below.

```bash
# Events for the sandbox pod (reason, message, count, time)
kubectl get events -n $NS --field-selector involvedObject.name=$POD \
  --sort-by=.lastTimestamp
# Per-container live CPU/memory
kubectl top pod $POD -n $NS --containers
```

## jsonpath cookbook (works regardless of the observability grant)

```bash
NS=moatless; TASK_ID=<uuid>
POD=$(kubectl get pod -n $NS -l moatless.io/task-id=$TASK_ID \
  --field-selector=status.phase=Running -o jsonpath='{.items[0].metadata.name}')

# OOM / crash: OOMKilled shows reason=OOMKilled, exit=137
kubectl get pod $POD -n $NS -o jsonpath='{range .status.containerStatuses[*]}{.name}: restarts={.restartCount} waiting={.state.waiting.reason} lastReason={.lastState.terminated.reason} exit={.lastState.terminated.exitCode}{"\n"}{end}'

# Eviction reason
kubectl get pod $POD -n $NS -o jsonpath='{.status.reason}{" "}{.status.message}{"\n"}'

# Resource limits (pair with `kubectl top pod` for OOM analysis)
kubectl get deploy moatless-ws-$TASK_ID -n $NS -o json \
  | jq '.spec.template.spec.containers[] | {name, resources}'
```

## Deriving the pod / deployment name for a task

- **Deployment & Service name is deterministic:** `moatless-ws-{task_id}` where
  `task_id` is sanitized — lowercased, `_`→`-`, truncated to 63 chars
  (`backend/src/sandbox/k8s/naming.rs`). Safe to construct for scale/patch.
- **The Pod name is NOT deterministic:** `moatless-ws-{task_id}-{rs-hash}-{pod-hash}`
  has random suffixes. **Never construct it — look it up by label.**
- **Labels on every sandbox pod:** `moatless.io/task-id=<task_id>`,
  `app.kubernetes.io/name=moatless-sandbox`, `app.kubernetes.io/component=sandbox`,
  `app.kubernetes.io/managed-by=moatless`.
- **Rolling-update gotcha:** during a redeploy two pods share the label. Filter
  by `--field-selector=status.phase=Running` (mirrors the backend's
  `pick_best_pod` in `pod_status.rs`) or you may grab the terminating pod.

`moat tasks …` commands accept a shortId directly; only `kubectl` needs the full
UUID. Get it from `moat tasks get` (which prints both `Task` and `ShortId`):

```bash
# shortId (from UI) -> full task_id
moat tasks get <shortId> --json | jq -r '.taskId'
# task_id -> pod name
kubectl get pod -n moatless -l moatless.io/task-id=$TASK_ID \
  --field-selector=status.phase=Running -o jsonpath='{.items[0].metadata.name}'
# task_id -> deployment name
echo "moatless-ws-$TASK_ID"
```

## Enabling `kubectl get events` + `kubectl top pods` from a sandbox

The Helm chart defines a `{{fullname}}-observability` Role granting
`events [get,list]` + `metrics.k8s.io/pods [get,list]`
(`k8s/helm/moatless-vibe/templates/rbac.yaml`), and — as of chart **0.1.1** —
binds it to the dedicated `kubernetes.sandboxServiceAccount` in addition to the
backend SA. So on an up-to-date release, `kubectl get events` and
`kubectl top pods` work from inside a sandbox.

> The grant is read-only (`get`/`list`); no mutation or escalation. `top pods`
> also requires metrics-server to be installed in the cluster.

If you still see `Forbidden`:
- You're on a release **older than chart 0.1.1** (the binding predates your
  deploy) — upgrade the chart, or bind the sandbox SA to the `-observability`
  Role manually.
- Fall back to the `moat` CLI in the meantime (`moat tasks diagnose` /
  `moat events list`) — the backend serves those with its own permissions.

## Still Forbidden from a sandbox

`kubectl get pvc`, `kubectl get role/rolebinding`, `secret` **list** (get by
name works), `kubectl exec`, `kubectl port-forward`, and deployment
create/delete remain denied to the `moatless-sandbox` SA by design. These need
an **admin kubeconfig** (or the backend SA). Prefer the `moat` CLI where it
covers the need — e.g. `moat tasks diagnose` surfaces PVC-related runtime events
without a direct `get pvc`, and `moat tasks logs` / `moat tasks files` replace
the need to `exec` into the pod.
