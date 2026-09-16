# Node disruption

A sandbox that disappeared with its node leaves nothing in its own namespace:
events for a cluster-scoped object are written to `default`, and the pod's own
last event is an ordinary termination. The reads below cross the sandbox
namespace, `default`, and the cluster-scoped Node and Karpenter objects — so
they need the ClusterRole every sandbox-access profile carries, not the
namespaced Role.

**Read the node the pod ran on, then that node's capacity type.**

```bash
kubectl -n <namespace> get pod <pod> -o jsonpath='{.spec.nodeName}'
kubectl get node <node> -o jsonpath='{.metadata.labels.karpenter\.sh/capacity-type}{"\n"}'
kubectl get node <node> -o jsonpath='{.spec.taints}{"\n"}'
```

`spot` only makes an interruption possible — Karpenter also terminates a spot
node for consolidation, drift or expiry — so establish the cause from the
NodeClaim conditions and the node events below. `on-demand` rules interruption
out and sends you back to the pod's own termination reason. A
`karpenter.sh/disrupted` taint means Karpenter, not the kubelet, started the
eviction.

**When the node is already gone, read the NodeClaim and the node events.**

```bash
kubectl get nodeclaims -o wide
kubectl get nodeclaim <name> -o yaml
kubectl get events -n default --field-selector involvedObject.kind=Node --sort-by=.lastTimestamp
```

Karpenter deletes the NodeClaim once the node finishes draining, so this is a
race with cleanup — collect it first. Read the reasons the events actually
carry rather than matching a list: interruption, consolidation and drift all
terminate a node and each implies a different repair.

**Read the NodePool before concluding anything about how long the pod had.**

```bash
kubectl get nodepool <name> -o yaml
```

`.spec.disruption` holds the consolidation policy and the budgets that decide
how many nodes may go at once; `.spec.template.spec.terminationGracePeriod`
holds the ceiling on drain time, which is what decides whether an agent's
in-flight work could have been checkpointed.

**Do not trust `kubectl auth can-i` for any of these.** It answers against the
current namespace, so a namespaced Role granting `*` returns `yes` for `nodes`
while the cluster-scoped request is still Forbidden. The tell is the warning it
prints alongside: `resource 'nodes' is not namespace scoped`.
