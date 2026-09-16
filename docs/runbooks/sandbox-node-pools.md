# Sandbox node pools

How a Workspace or a Loop puts its sandbox pods on dedicated capacity, and what
the node pool behind that has to look like. The pool itself is not defined here:
the NodePool and EC2NodeClass live in `soap-infra` /
`soap-platform-gitops`, and the manifests below are the reference this
repository's code was written against.

## What the platform sends to Kubernetes

Every sandbox pod carries the deployment-wide selector and tolerations from Helm
values:

```yaml
kubernetes:
  sandboxNodeSelector: { workload: moatless }
  sandboxTolerations:
    - key: workload
      operator: Equal
      value: moatless
      effect: NoSchedule
```

On top of that, two rows can add labels:

- `workspaces.node_selector` — every sandbox opened in that Workspace.
- `loop_configs.node_selector` — every sandbox opened by that Loop. Outranks the
  Workspace's value.

Both are **merged over** `sandboxNodeSelector`, never substituted for it: the
deployment-wide labels are what pair with the deployment-wide toleration, so a
pod that drops them no longer matches the taint it is meant to land on. The
merge happens in `crates/sandbox-k8s/src/pod_builder/pod.rs`, and precedence is
resolved once when the `sandboxes` row is written
(`crates/tasks/src/task/dao.rs`), so rebuilding a pod keeps the placement its
Sandbox started with.

Both columns are admin-API-only. Neither `.moatless/workspaces.json` nor
`LOOP.md` can set one — commit access would otherwise be enough to move a
repository's work onto more expensive capacity.

## What a new pool has to carry

A pool that sandbox pods can reach must have:

1. **The deployment-wide label**, `workload: moatless` — the global selector
   still applies.
2. **The matching taint**, `workload=moatless:NoSchedule` — so nothing else
   drifts onto it.
3. **One distinguishing label** the per-Workspace/per-Loop selector names, e.g.
   `moatless.io/pool: on-demand`.

The distinguishing label is what the `--node-selector` value below matches. It is
a label on the pool rather than `karpenter.sh/capacity-type`, so the pool's
capacity type, instance families and disk stay an infra decision that no
application row has to restate.

## Reference manifests

```yaml
apiVersion: karpenter.sh/v1
kind: NodePool
metadata:
  name: moatless-ondemand
spec:
  template:
    metadata:
      labels:
        workload: moatless
        moatless.io/pool: on-demand
    spec:
      taints:
        - key: workload
          value: moatless
          effect: NoSchedule
      requirements:
        - key: karpenter.sh/capacity-type
          operator: In
          values: ["on-demand"]
        - key: kubernetes.io/arch
          operator: In
          values: ["amd64"]
        - key: karpenter.k8s.aws/instance-category
          operator: In
          values: ["c", "m", "r"]
        - key: karpenter.k8s.aws/instance-cpu
          operator: In
          values: ["4", "8", "16"]
      nodeClassRef:
        group: karpenter.k8s.aws
        kind: EC2NodeClass
        name: moatless-ondemand
      # An autonomous run can outlast a rotation window, and a drain mid-run is
      # the interruption this pool exists to avoid.
      expireAfter: Never
  disruption:
    # The pool is sized by what is running on it, so a node with no sandbox left
    # is billed for nothing. Pairs with `stopWhenDone` below.
    consolidationPolicy: WhenEmpty
    consolidateAfter: 1m
  limits:
    cpu: "64"
---
apiVersion: karpenter.k8s.aws/v1
kind: EC2NodeClass
metadata:
  name: moatless-ondemand
spec:
  # Match the spot sandbox pool (`moatless-egress`) on everything but capacity
  # type: same AMI family, role, subnets and security groups.
  amiFamily: AL2023
  amiSelectorTerms:
    - alias: al2023@latest
  role: <same node role as the spot sandbox pool>
  subnetSelectorTerms:
    - tags:
        karpenter.sh/discovery: <cluster>
  securityGroupSelectorTerms:
    - tags:
        karpenter.sh/discovery: <cluster>
  blockDeviceMappings:
    - deviceName: /dev/xvda
      ebs:
        volumeSize: 200Gi
        volumeType: gp3
        encrypted: true
        deleteOnTermination: true
  userData: |
    #!/bin/bash
    # Sandbox pods exhaust both defaults during frontend builds.
    sysctl -w fs.inotify.max_user_watches=524288
    sysctl -w vm.max_map_count=1048576
```

## Putting a Loop on it

```bash
moat loops update <loop-id> \
  --node-selector '{"moatless.io/pool":"on-demand"}' \
  --stop-when-done true
```

Or for every sandbox in a Workspace:

```bash
moat workspaces update <workspace-id> \
  --node-selector-json '{"moatless.io/pool":"on-demand"}'
```

`--clear-node-selector` (loops) and `--node-selector-json '{}'` (workspaces)
put the sandboxes back on whatever the deployment-wide selector resolves to.

## `stopWhenDone`

`loop_configs.stop_when_done` stops the Loop's sandbox as soon as its agent run
finishes, instead of holding the pod until the idle TTL expires. Pair it with
dedicated capacity: the pool's `consolidationPolicy: WhenEmpty` cannot release a
node while a pod is still sitting on it, so the idle TTL would otherwise be the
real bill.

The stop is skipped while the sandbox still has a registered command running
(`moat cmd run`) or another active member Task, and an errored run keeps its pod
for the idle TTL so the failure stays inspectable. In every skipped case the idle
reaper is still what stops the pod.

Unlike `node_selector`, this one is declarable in `LOOP.md` frontmatter
(`stopWhenDone: true`) — it only shortens a sandbox's life, so commit access
buys no capacity.

## Why this exists

Sandbox pods run on a spot pool, and a reclaim mid-turn kills the agent's
process. That is tolerable for an interactive task, where a person retries; it is
not for an autonomous Loop whose whole run is unattended. Pinning those Loops to
on-demand capacity trades cost for the reclaim, and `stopWhenDone` pays most of
that cost back by releasing the node the moment the run ends.
