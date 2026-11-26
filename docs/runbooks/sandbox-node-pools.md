# Sandbox node pools

How a Workspace or a Loop puts its sandbox pods on dedicated capacity, and what
the node pool behind that has to look like. The pool itself is not defined here:
the NodePool and its NodeClass belong to the cluster's infrastructure, and the
tolerations that reach it to the deployment's Helm values.

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

Both are **merged over** `sandboxNodeSelector` **key by key**, never substituted
for it whole: a deployment-wide key the row leaves out survives, and one the row
names is replaced. Replacing a key is the point — it is how a row reaches a pool
the deployment-wide selector does not, and the section below turns on it.

Tolerations are configured separately and are not derived from the selector, so
changing a label neither removes nor alters what the pod tolerates: every
sandbox carries every entry in `sandboxTolerations`. The pool a row selects
still needs a matching entry there, which is why the list below asks for one.

The merge happens in `crates/sandbox-k8s/src/pod_builder/pod.rs`, and precedence
is resolved once when the `sandboxes` row is written
(`crates/tasks/src/task/dao.rs`), so rebuilding a pod keeps the placement its
Sandbox started with.

Both columns are admin-API-only. Neither `.moatless/workspaces.json` nor
`LOOP.md` can set one — commit access would otherwise be enough to move a
repository's work onto more expensive capacity.

## An example pool

An on-demand pool for unattended Loops: the same NodeClass, instance shape and
zone as the spot sandbox pool, so an opted-in sandbox gets the same machine and
egress path, labelled and tainted `workload: moatless-od` and reached with
`--node-selector '{"workload":"moatless-od"}'`.

## What a new pool has to carry

1. **A distinguishing value on the `workload` key** — `workload: moatless-od`,
   not a second label beside `workload: moatless`. Because the merge is
   key-by-key, naming `workload` in a row's selector replaces the
   deployment-wide value, and that replacement is the only thing keeping a
   sandbox that did *not* opt in off the pool's nodes. A pool that keeps
   `workload: moatless` and adds a second label is reachable but not reserved:
   every other sandbox still matches it and can be bin-packed onto it.
2. **The matching taint**, `workload=<value>:NoSchedule`, so nothing outside
   moatless drifts onto it.
3. **That taint in `kubernetes.sandboxTolerations`**
   in the deployment's Helm values. Tolerations
   are deployment-wide and additive, and one places nothing on its own, so it is
   safe to add before the pool exists — and it has to be there before any row
   selects the pool, or those sandboxes sit Pending on the taint.

Capacity type stays out of the selector: it is a label on the pool rather than
`karpenter.sh/capacity-type`, so instance families, disk and expiry stay an infra
decision no application row restates.

### What a dedicated pool does not get

`image-prepull` and `node-storage-janitor` are DaemonSets that take
`kubernetes.sandboxNodeSelector`, still `workload: moatless`, so neither lands on
a pool with a different `workload` value. A fresh node there pulls the sandbox
image cold on its first run and runs no hostPath janitor — see
`docs/deferred-changes.md`.

## Putting a Loop on it

```bash
moat loops update <loop-id> \
  --node-selector '{"workload":"moatless-od"}' \
  --stop-when-done true
```

Or for every sandbox in a Workspace:

```bash
moat workspaces update <workspace-id> \
  --node-selector-json '{"workload":"moatless-od"}'
```

`--clear-node-selector` (loops) and `--node-selector-json '{}'` (workspaces)
put the sandboxes back on whatever the deployment-wide selector resolves to.

A Git-synced Loop rejects both flags, and `LOOP.md` has no field for a selector.
The Workspace command above is the way out **only while that Workspace is
`source: manual`** — `moat workspaces update` refuses a Git-synced one for the
same reason (`ensure_workspace_mutable`). Check with
`moat workspaces get <id> --json`.

When both are Git-owned, nothing sets the selector without taking one of them
off Git sync. Prefer `moat workspaces override`: it keeps the provenance fields
and `moat workspaces reset-git` puts it back, whereas `moat loops override`
freezes the Loop's prompt and schedule as well.

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
