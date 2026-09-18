---
name: workspace-resource-tuning
description: Right-size Moatless sandbox CPU and memory — decide which workspaces need more memory, why the sandbox pool is running more nodes than it should, and how to reclaim phantom reservations. Measures with Grafana RSS metrics, attributes usage to workspaces, then applies via the moat CLI and a redeploy.
---

# Workspace Resource Tuning

Size sandbox CPU and memory from what sandboxes actually use. The measurements
here are unusually easy to misread — the default metric overstates need by up to
10×, and the obvious way to join metrics to workspaces silently produces wrong
answers. Each rule below carries the reason, because knowing *why* is what lets
you adapt when the numbers look different from the examples.

For the reactive case — a task is stuck, a sandbox was OOMKilled or evicted, a
pod won't start — use the `sandbox-troubleshooting` skill instead.

Copy-pasteable queries: [references/queries.md](references/queries.md).

## Get this right first: requests reserve, limits don't

This distinction decides whether your recommendation saves money or wastes it.

**Requests reserve capacity.** The scheduler bin-packs on requests, so they
determine how many sandboxes fit per node and therefore how many nodes exist.
A request is a standing claim held whether or not the sandbox uses it. **This is
the only knob that costs money.**

**Limits only cap.** A limit reserves nothing. An 8Gi limit on a container using
133Mi costs exactly zero — it just sets where the OOM killer steps in. Raising a
limit reclaims nothing; lowering one saves nothing.

So the two are sized from different statistics:

| | Sized from | Because |
|---|---|---|
| Request | **median** usage | It is held continuously, so it should reflect the typical state |
| Limit | **peak** usage × ~1.5 | It only matters at the peak, and headroom is free |

The most expensive mistake available is inverting this — sizing requests from
peaks (reserving burst capacity permanently, which multiplies node count) or
"saving money" by lowering limits (which saves nothing and OOM-kills agents
mid-turn). If you catch yourself proposing a fleet-wide limit change for cost
reasons, stop; there is nothing to reclaim there.

## 1. Measure

Run `scripts/workspace-report.sh` for the standard pass. It does the whole
measure → attribute → verdict pipeline with the joins done correctly. Reach for
the raw queries only when digging past what the report shows.

The rules the report encodes, and why they matter if you query by hand:

**Use `container_memory_rss`, not `container_memory_working_set_bytes`.**
Working set includes page cache, which is reclaimable — the kernel drops it
under pressure rather than OOM-killing. On an idle sandbox, cache has been 93%
of working set (189Mi RSS inside a 2602Mi working set). Sizing anything on that
number over-provisions wildly.

**Never query an instant value.** Use `max_over_time(...[24h])` for peaks and
`quantile_over_time(0.5, ...[24h])` for typical. Sandboxes are idle the large
majority of the time, so an instant query almost always samples an idle fleet
and tells you sandboxes need a few hundred MiB. They don't — under load the same
fleet reached 6.8 GiB RSS. Both numbers are real; only one is a peak.

**Absent `container_cpu_cfs_*` series mean no CPU quota is set.** These only
exist where a limit exists, so a missing series is a finding — that container is
unbounded — not a gap in the data.

**Read throttling as a ratio**, `cfs_throttled_seconds / cfs_periods`. Anything
under a few percent means the CPU limit is not costing latency and does not need
raising.

## 2. Attribute usage to workspaces

Two traps here, both of which produce confident wrong answers.

**Pod names encode the task ID, not the workspace ID.**
`moatless-ws-<uuid>-<hash>` — that uuid is `moatless.io/task-id`. Map it through
`moat tasks list --json`, which gives `taskId` → `workspaceId`.

**Join on workspace ID, never workspace name.** Names are not unique. The same
name can exist several times with *different* limits, so a name join picks an
arbitrary one and silently reports the wrong limit — flipping a workspace's
verdict between "fine" and "needs raising". If a report shows duplicate names
with different numbers, that is the signal the join is keyed wrongly.

## 3. Decide

| Observation | Action |
|---|---|
| peak > 85% of limit | Raise the limit (headroom is free; an OOM mid-turn is not) |
| median RSS > request + 10% | Raise the request to the median — the scheduler is packing this node on a number the workspace exceeds continuously |
| request > ~4× median RSS | Lower the request toward the 512Mi floor |
| limit ≫ peak on a light workspace | Leave it — limits cost nothing |
| throttle ratio < ~5% | CPU limit is fine, don't touch it |

Two things worth checking before recommending a change:

- **Is this workspace's usage explained by what it builds?** Repos that compile
  large native toolchains sit an order of magnitude above app repos. Group by
  what the workspace builds rather than treating each as independent — it makes
  outliers obvious and predicts workspaces that haven't been measured yet.
- **How many tasks did the sample cover?** A peak drawn from two tasks is a
  lower bound, not a ceiling. Say so rather than presenting it as settled.

The statistical justification for the 512Mi request floor is in
[references/queries.md](references/queries.md) — useful when someone challenges
the number, unnecessary for a routine pass.

## 4. Apply

Confirm the change set with the operator before mutating anything.

**`--resources-json` replaces the entire resources block.** `ResourceConfig`
doubles as the *storage* config — it carries `storage`, `buildCacheMode`,
`buildCacheScope`, `dockerStorage` and `dockerStorageMode` alongside
`requests`/`limits`. Sending only the fields you want to change silently wipes
the rest. Always read-modify-write the whole block:

```bash
NEW=$(jq -c --arg id "$ID" '.[] | select(.id==$id) | .resources
        | .requests.memory="512Mi"' workspaces.json)
moat workspaces update "$ID" --resources-json "$NEW"
```

**`update` only works on `source=manual` workspaces.** Git-synced ones need
`moat workspaces override`. Check `source` and `syncedFromGit` first.

**Verify by re-reading the API.** The CLI reports success in cases that did not
take effect, so confirm the new values — and confirm the storage fields survived
— with a fresh `moat workspaces list --json`. Soft-deleted duplicates reject
updates with `409 Workspace has been deleted`; they can't spawn sandboxes, so
they're harmless to skip, but they do inflate any fleet-wide total you report.

## 5. Redeploy — the step that's easy to miss

**Nothing re-renders an existing Deployment.** A workspace config change only
affects Deployments created *after* it. Existing ones keep their original pod
template indefinitely, and a pod restart faithfully reproduces the old spec.

This reliably generates false alarms: a pod created hours after a fix ships
still shows the old values, which looks exactly like the fix not working. Before
concluding that, diff the Deployment template against the workspace config. If
they disagree, the Deployment is stale and the fix is fine.

It also means a config-only change reclaims nothing until the redeploy happens.
The reaper scales idle sandboxes to zero replicas, so most Deployments are
usually asleep — redeploying those is free and disrupts nobody. That window
closes as people start working, so it's worth doing promptly.

After redeploying, confirm on the cluster rather than in config: node count,
requests reserved per node, and that new pods carry the expected requests.

**Confirm that through Prometheus, not `kubectl get nodes`.** The
`moatless-admin` service account cannot list nodes cluster-wide, so
`kubectl get nodes` and `kubectl describe node` both fail with a forbidden
error; `kube_node_status_allocatable`, `kube_pod_container_resource_requests`
and `kube_node_labels` carry the same numbers and do work from here. See
[Node allocation](references/queries.md#node-allocation).
