# Query recipes

Ad-hoc queries for digging past what `scripts/workspace-report.sh` reports.

- [Grafana wrapper](#grafana-wrapper)
- [Memory: RSS vs working set](#memory-rss-vs-working-set)
- [CPU throttling](#cpu-throttling)
- [Concurrency and duty cycle](#concurrency-and-duty-cycle)
- [Node allocation](#node-allocation)
- [Deployment drift](#deployment-drift)
- [OOM, restarts, evictions](#oom-restarts-evictions)
- [Why 512Mi](#why-512mi)

## Grafana wrapper

Everything below assumes this helper. In a sandbox `GRAFANA_URL` is the relay,
which injects the credential, so send no `Authorization` header. On a laptop,
set `G` to the workspace URL and add `-H "Authorization: Bearer $GRAFANA_TOKEN"`
with the token from `.envrc`.

```bash
G=$GRAFANA_URL
q() { curl -s -G "$G/api/datasources/proxy/uid/prometheus/api/v1/query" \
        --data-urlencode "query=$1"; }
```

The datasource proxy returns native Prometheus JSON, which is far easier to
parse than Grafana's `/api/ds/query` frame format. Use `query_range` with
`start`/`end`/`step` for trends.

Check how much history a metric actually has before trusting a `[24h]` window —
a recently-added metric silently returns a shorter window:

```bash
q 'count_over_time(sum(container_memory_rss{namespace="moatless",container="sandbox"})[48h:1m])'
```

Per-series sample counts reflect *pod lifetime*, not retention — pods churn, so
aggregate first as above.

## Memory: RSS vs working set

Peak and typical per pod:

```bash
q 'max_over_time(container_memory_rss{namespace="moatless",container="sandbox"}[24h])'
q 'quantile_over_time(0.5, container_memory_rss{namespace="moatless",container="sandbox"}[24h])'
```

The page-cache gap — how much of working set is reclaimable. Join in PromQL
rather than pasting two shell outputs together; separately-sorted lists
misalign when the two queries return different pod sets:

```bash
q 'container_memory_working_set_bytes{namespace="moatless",container="sandbox"}
   - on(pod) container_memory_rss{namespace="moatless",container="sandbox"}'
```

A large gap at idle is normal and not a sizing signal. Under load the gap
narrows and RSS becomes the bulk — that is the number limits must cover.

## CPU throttling

Throttling is a ratio, not a count:

```bash
q 'sum by (pod) (rate(container_cpu_cfs_throttled_seconds_total{namespace="moatless",container="sandbox"}[10m]))
 / sum by (pod) (rate(container_cpu_cfs_periods_total{namespace="moatless",container="sandbox"}[10m]))'
```

Peak over a window, via subquery:

```bash
q 'topk(10, max_over_time( (
     sum by (pod) (rate(container_cpu_cfs_throttled_seconds_total{namespace="moatless",container="sandbox"}[5m]))
   / sum by (pod) (rate(container_cpu_cfs_periods_total{namespace="moatless",container="sandbox"}[5m]))
   )[24h:5m] ))'
```

Sanity-check the metric against a known-throttled workload elsewhere in the
cluster before believing a fleet-wide zero. A pod with **no** `cfs_periods`
series has no CPU quota at all — that's an unbounded container, worth reporting.

These three series are retained by an explicit keep-list in the nonprod
Prometheus values (`apps/nonprod/prometheus/values.yaml` in
`soap-platform-gitops`). If they return nothing cluster-wide, check that
keep-list before concluding the workload isn't throttling.

## Concurrency and duty cycle

How many sandboxes run at once — this decides whether density matters at all:

```bash
q 'max_over_time(count(container_memory_rss{namespace="moatless",container="sandbox"})[24h:1m])'
q 'avg_over_time(count(container_memory_rss{namespace="moatless",container="sandbox"})[24h:1m])'
```

Fraction of time above a threshold (`> bool` yields 1/0, so `avg_over_time` of
it is the duty cycle):

```bash
q 'avg(avg_over_time((container_memory_rss{namespace="moatless",container="sandbox"} > bool 4294967296)[24h:1m]))'
```

## Node allocation

Requests reserved versus capacity — the direct read on whether requests are
driving node count.

**Use these Prometheus queries, not `kubectl get nodes`.** The `moatless-admin`
service account holds no cluster-wide node permission, so `kubectl get nodes`
and `kubectl describe node` both fail with a forbidden error. `kube-state-metrics`
publishes the same numbers:

```bash
q 'count(kube_node_labels{label_workload="moatless"})'                  # pool size

q 'kube_node_status_allocatable{resource="memory"}
   * on(node) group_left() kube_node_labels{label_workload="moatless"}' # capacity

q 'sum by (node) (kube_pod_container_resource_requests{namespace="moatless",resource="memory"})'
q 'sum by (node) (kube_pod_container_resource_requests{namespace="moatless",resource="cpu"})'
```

The reserved fraction — what `kubectl describe node` prints under `Allocated
resources`:

```bash
q 'sum by (node) (kube_pod_container_resource_requests{namespace="moatless",resource="memory"})
 / on(node) group_left() kube_node_status_allocatable{resource="memory"}'
```

`kube_node_status_allocatable` carries one series per `resource`, so filter it —
unfiltered, the division matches `cpu` against `memory` and returns nonsense.

A node showing high CPU-requested with few sandboxes on it is the signature of
stale templates carrying inflated CPU requests.

## Deployment drift

Whether running Deployments match current workspace config. Divergence here
explains "the fix didn't work":

```bash
kubectl -n moatless get deploy -l app.kubernetes.io/component=sandbox \
  -o jsonpath='{range .items[*]}{.metadata.labels.moatless\.io/task-id}{"\t"}{.spec.template.spec.containers[0].resources.requests.cpu}{"\t"}{.spec.template.spec.containers[0].resources.requests.memory}{"\t"}{.spec.template.spec.containers[0].resources.limits.memory}{"\n"}{end}'
```

Distribution of template values across all Deployments — the quickest read on
how many are stale:

```bash
kubectl -n moatless get deploy -l app.kubernetes.io/component=sandbox \
  -o jsonpath='{range .items[*]}{.spec.template.spec.containers[0].resources.requests.cpu}{"\n"}{end}' \
  | sed 's/^$/NONE/' | sort | uniq -c | sort -rn
```

Sorting Deployments by `.metadata.creationTimestamp` shows a clean before/after
boundary when a backend fix changed how templates are rendered.

Replica counts — how many are asleep and therefore free to redeploy:

```bash
kubectl -n moatless get deploy -l app.kubernetes.io/component=sandbox \
  -o jsonpath='{range .items[*]}{.spec.replicas}{"\n"}{end}' | sort | uniq -c
```

## OOM, restarts, evictions

```bash
kubectl -n moatless get pods -l app.kubernetes.io/component=sandbox -o json \
 | jq -r '.items[] | .metadata.name as $n | .status.containerStatuses[]?
          | select(.name=="sandbox")
          | "\($n) restarts=\(.restartCount) last=\(.lastState.terminated.reason // "-")"'

kubectl get events -A | grep -iE 'oom|evict'
```

Note that repeated pod instances for one workspace in a metrics window are
usually node consolidation, not crash loops — confirm against `restartCount`
before calling it a crash.

## Why 512Mi

The request floor comes from treating per-sandbox memory as a two-state
distribution — idle most of the time, bursting occasionally — and asking what
per-sandbox reservation keeps a node safe when N sandboxes share it.

With mean μ and standard deviation σ per sandbox, N sandboxes on a node sum to
mean `Nμ` and spread `√N·σ`. Staying within capacity at roughly three standard
deviations requires:

```
r ≥ μ + 3σ/√N
```

Measured on RSS (μ ≈ 0.2 GiB, σ ≈ 0.43 GiB), this gives ~0.53 GiB at N=15 and
~0.44 GiB at N=30 — converging on 512Mi, and self-consistent since 512Mi
requests are what produce those N values.

Two things to note when reusing this:

- Run it on **RSS**. The same derivation on working set gave ~2Gi, roughly 4×
  too high, because page cache inflated both μ and σ.
- The `/√N` term means larger nodes need *less* per-sandbox margin — burst
  headroom pools. That argues for fewer, larger nodes when concurrency is high
  enough for density to matter at all.
