# Queries

Every check `collect.mjs` runs, in the form that returns the right answer, plus the ones worth running by hand when a fix needs more than the digest. Numbers in the examples are from 2026-09-16, the day these were verified against a published report.

## Setup

```bash
# In a sandbox: the relay injects the credential, so send no Authorization header.
G=$GRAFANA_URL; AUTH=()
# On a laptop instead (GRAFANA_TOKEN from .envrc):
# G=https://g-0000000000.grafana-workspace.eu-central-1.amazonaws.com
# AUTH=(-H "Authorization: Bearer $GRAFANA_TOKEN")
L="$G/api/datasources/proxy/uid/loki/loki/api/v1/query_range"
P="$G/api/datasources/proxy/uid/prometheus/api/v1"
D=2026-09-16
DAY0=$(date -u -d "$D 00:00:00" +%s); DAY1=$((DAY0+86400))
```

Datasource uid `loki`/`prometheus`/`tempo` is the nonprod workspace; moatless staging lives there under `namespace="moatless"`. The `-prod` uids are a different environment, and `namespace="soap"` on either is a different product.

## Window and bucket labelling

A `count_over_time(...[1h])` sample at `t` covers `(t-1h, t]`. So the day's 24 buckets are the samples from `DAY0+3600` through `DAY1` inclusive, and the sample at `t` belongs to hour `UTC(t) - 1`:

```
--data-urlencode "start=$((DAY0+3600))" --data-urlencode "end=$DAY1" --data-urlencode "step=3600"
```

Starting at `DAY0` returns 25 samples whose first belongs to the previous day. `hourly()` in `collect.mjs` does the subtraction; a hand-run query needs it too.

Pass `direction=backward` on any query that returns raw lines rather than a matrix, or hitting `limit` gives you the oldest lines in the window instead of the newest.

## Kubernetes warning events

```
sum by (reason) (count_over_time({namespace="moatless", service_name="integrations/kubernetes/eventhandler"}
  | regexp `reason=(?P<reason>\S+) type=(?P<etype>\S+)` | etype="Warning" [1h]))
```

The stream carries `Normal` events too. Without `etype="Warning"` the total roughly quadruples and reads as a catastrophe. The `reason` label only exists after the `regexp` stage — `sum by (reason)` on the bare stream returns one ungrouped series and no error.

Verified: 2,066 warnings, `FailedScheduling` 1,045, `Unhealthy` 757, `NodeNotReady` 183, `BackOff` 77.

## Sandbox pods

```
count by (pod) (count_over_time({namespace="moatless", service_name="moatless-sandbox"}[1h]))
```

Keep the `pod` label rather than collapsing it with an outer `count()`: one query then answers both pod questions. Counting the series in each bucket gives concurrency — a lower bound, since a pod alive but silent for an hour is not counted — and the pod names themselves give pod churn. A pod is `moatless-ws-<task-id>-<hash>`, so the uuid groups every pod that served one sandbox:

```js
const id = s.metric.pod.match(/^moatless-ws-([0-9a-f-]{36})/)?.[1] ?? s.metric.pod;
```

`digest.sandboxPods` carries `pods`, `sandboxes`, `multiPod`, `peakConcurrent` and `podsPerHour`. Verified 2026-09-16: 346 pods across 116 sandboxes, peak 54. On 2026-09-17: 300 across 172, peak 58 — the ratio is what moved, not the pod count.

## Sandbox disruptions

Two causes restart a sandbox under running work, and the pod count alone cannot tell them apart. Spot reclaims come from `NodeClaim` events, which carry no `namespace` label, so the sandbox pools are matched by name:

```
sum(count_over_time({service_name="integrations/kubernetes/eventhandler"} |~ "kind=NodeClaim "
  |~ "name=moatless-egress-(fallback-)?[a-z0-9]{5} " |~ "reason=SpotInterrupted" [1h]))
```

Evictions are read as lines (`reason=Evicted`, `name=moatless-ws-`), because the cause is in the message: `Usage of EmptyDir volume "workspace-data" exceeds the limit "10Gi"` is the workspace's `resources.storage`, not node pressure. Every pod on a reclaimed node also logs a `NodeNotReady` warning, so that reason's peaks follow the reclaims.

`digest.sandboxDisruptions` carries `spotReclaims`, `spotPerHour`, `evictions`, `evictedForVolume`, `evictedTasks` and `evictedPerHour`. Verified 2026-10-08: 17 reclaims, 12 evictions on 6 tasks, all for the volume limit.

## Log volume by level and service

```
sum by (log_level) (count_over_time({namespace="moatless", service_name="moatless"}[1h]))
sum by (log_level) (count_over_time({namespace="moatless", service_name="moatless-sandbox"}[1h]))
```

`log_level` is a stream label here, so no parser stage is needed. Backend `ERROR` has been the tile-worthy number; sandbox `WARN` runs in the millions and is almost entirely the noise patterns below.

## HTTP 5xx by path

```
sum by (path, status) (count_over_time({namespace="moatless", service_name="moatless"}
  | json path="span.path", status="span.status" | status =~ "5[0-9][0-9]" [1h]))
```

`| json | status >= 500` fails the whole query with `LabelFilterErr … strconv.ParseFloat: parsing "": invalid syntax`, because every non-HTTP line parses `status` as empty. Both fields are nested under `span`, so they need the explicit extraction shown; the regex match then keeps it a string comparison. Verified: 34 responses over 20 path/status pairs.

## Noise patterns

```
sum(count_over_time({namespace="moatless"} |= "<needle>" [1h]))
```

The needles `collect.mjs` tracks, and what each means:

| Key | Needle | Reads as |
|---|---|---|
| `rustic_xattr` | `setting extended attributes failed` | Backup writing to a filesystem without xattr support |
| `cli_unknown_frame` | `CLI stdout: unknown type` | An agent CLI frame the harness does not model |
| `review_git_failed` | `git command for the review surface failed` | Review surface could not read the repo |
| `system_time_zone` | `failed to get system time zone` | Missing tzdata in a container |
| `slow_statement` | `slow statement` | A query past the backend's threshold |
| `sandbox_slow_start` | `Sandbox container slow to start` | Provisioning latency the agent waited on |

Add a needle when a fix needs its rate over the day; a one-off count does not need to live in the script.

## Reading raw lines behind a count

Once a count points somewhere, read the lines. This is what turns a metric into a fix worth writing up:

```bash
curl -s -G "$L" ${AUTH[@]+"${AUTH[@]}"} \
  --data-urlencode 'query={namespace="moatless", service_name="moatless"} |= "preflight"' \
  --data-urlencode "start=$DAY0" --data-urlencode "end=$DAY1" \
  --data-urlencode "limit=200" --data-urlencode "direction=backward" \
  | jq -r '.data.result[].values[] | "\(.[0]|tonumber/1e9|todate) \(.[1])"' | sort
```

When a query returns nothing, check the plain-text error before believing it: `curl -w 'HTTP %{http_code}'` and `head -c 400`. Piping a Loki pipeline error into a JSON parser turns it into a decode failure that hides the real message.

## Moatless REST

`moat api GET <path> --json` returns `{status, body}`. The collector walks these:

| Path | Used for |
|---|---|
| `/api/v1/tasks?limit=200&offset=N` | Created, touched, status, `totalCostUsd`, repo, agent profile, `bindings[].loopId` |
| `/api/v1/tasks/<id>/statistics` | Token usage with timestamps, one entry per Claude turn from the run's result, and `toolCalls` with `failedCalls` |
| `/api/v1/tasks/<id>/messages?summary=true&limit=200&before_id=N` | Agent messages per hour: the `messageType: "assistant"` rows, paged back until one is older than the window |
| `/api/v1/feedback` | Feedback rows with `category` and `createdAt` |
| `/api/v1/loops` | Which Loops exist and when each last fired |
| `/api/v1/workspaces`, `/api/v1/repositories` | Names and resource config for attribution |

Five things about this data:

- **A task row keys on `taskId`, not `id`.** `Object.fromEntries(tasks.map(t => [t.taskId, t]))` is the index; keying on `id` returns undefined for every lookup and reads as the task being absent.
- **`bindings[].loopId` is what attributes a task to a Loop**, and only an `adapter_event` loop sets it. A scheduled loop's task carries no binding, so a "Loops that fired" table covers the adapter loops and the schedules sit in the unattributed remainder — say so under the table rather than presenting the count as every loop.
- **`/api/v1/tasks` is newest-first and uncapped in the API**, so the collector stops at `CAP` rows. It warns only when the oldest row pulled is newer than the day being collected, which is the case where the day is actually truncated.
- **`status` and `totalCostUsd` are current values, not end-of-day values.** Re-running the collector a day later gives different closed counts. Date the collection in the method note.
- **Statistics calls rate-limit above about 8 concurrent.** `pool(items, 8, …)` is why the run takes minutes rather than seconds.

## Prometheus, when a fix needs resource numbers

Memory and CPU sizing belong to the `workspace-resource-tuning` skill, which has the joins and the statistics right. Two rules carry over if a health-report fix needs one number from there: read `container_memory_rss`, not working set, and never take an instant value — `max_over_time(...[24h])` for peaks, `quantile_over_time(0.5, ...[24h])` for typical.
