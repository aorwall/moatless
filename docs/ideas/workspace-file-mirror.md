# Mirror the workspace to S3 as files, not as one tar

An idea with its cost measured, not a plan. Measurements are from the staging
fleet on 2026-09-04. Re-run them with `scripts/workspace-s3-cost.py` before you
act on the numbers.

## What the sandbox does today

`sandbox/src/init/workspace_sync.rs` builds one gzipped tar of `WORKSPACE_DIR`
and PUTs it to `<prefix>/<task_id>/workspace.tar`. The backend mints presigned
PUT/GET URLs (`crates/backend/src/sandbox/grpc_server.rs:1911`) and stays out of
the data path, so the sandbox holds no AWS credentials. The key builder is
`moatless_sandbox_runtime::storage::sandbox_key`.

Four paths upload, and all four share one fingerprint gate
(`WorkspaceSyncTracker`), so an unchanged workspace ships nothing: the turn
boundary, the pre-stop `SyncWorkspace` RPC, the shutdown flush, and a 5-minute
periodic loop. The periodic loop runs only when the volume is not durable
(`is_periodic_enabled`). Staging sets `K8S_SANDBOX_DEFAULT_PVC=false`, so every
sandbox there runs on an `emptyDir` and the S3 archive is its only durable copy.

The archive carries, per repo, the working-tree set from `git ls-files -c -o
--exclude-standard` plus the whole `.git`, and every loose file outside a repo.
Gitignored artifacts stay out.

## What the tar cannot do

Every file read goes to the live pod. `read_file_handler`, `list_files_handler`
and `file_tree_handler` in `crates/sandbox-control/src/file_handlers.rs` each
call `state.connector.sandbox_client(&task_id)` first, so a stopped sandbox
answers 503. The bytes are in S3, inside a tar nothing can address a member of.
A per-file mirror is what makes a stopped task's workspace readable.

## How to measure the cost

Three sources, all read-only.

**Fleet rates from Prometheus.** The backend records every sync the sandbox
reports (`moatless_workspace_sync_*`, `crates/sandbox-runtime/src/metrics.rs`).
The Grafana workspace scrapes it.

```bash
G=https://g-0000000000.grafana-workspace.eu-central-1.amazonaws.com
curl -s -G "$G/api/datasources/proxy/uid/prometheus/api/v1/query" \
  -H "Authorization: Bearer $GRAFANA_TOKEN" \
  --data-urlencode 'query=sum by (direction,outcome) (increase(moatless_workspace_sync_total[7d]))'
# mean archive size: _bytes_sum / _bytes_count, direction="upload"
```

**The file set from a live pod.** `scripts/workspace-s3-cost.py measure` walks
the same set `build_tar` archives and reports file count, bytes, the `.git`
split, a size histogram, and how much of the set changed inside 300s, 1h and
24h. Pipe it into any sandbox — the pods run python3, the walk takes under a
second, and `--tar` adds the gzip pass:

```bash
kubectl exec -n moatless <pod> -c sandbox -i -- \
  python3 - measure /opt/moatless/workspace --tar < scripts/workspace-s3-cost.py
```

**The bill from the measurement.** `cost` prices three layouts against the fleet
rates:

```bash
scripts/workspace-s3-cost.py cost ws.json --workspaces 6974 \
  --uploads-per-day 564 --downloads-per-day 144 \
  --mean-tar-gz-bytes 87411705 --min-churn-files 12 --new-workspaces-per-month 350
```

## What the fleet does

Seven days, staging:

| Measure | Value |
|---|---|
| Uploads | 3,946 ok, 20 failed (564/day) |
| Downloads | 1,006 ok, 0 failed (144/day) |
| Bytes uploaded | 344.9 GB (49 GB/day) |
| Mean archive | 87.4 MB gzipped |
| Tasks (all time) | 6,974 |

Nine live sandboxes, sampled one time each:

| Measure | Range | Median |
|---|---|---|
| Files in the archive set | 710 – 6,315 | 2,712 |
| Raw bytes | 49 MB – 373 MB | 66 MB |
| Files under 4 KB | 14% – 69% | 62% |
| `.git` files | 29 – 1,672 | 29 |
| Files changed in the last hour | 0 – 12 | 1 |
| Compression ratio, 3 samples (raw / gzipped) | 1.4x – 2.6x | 1.9x |

Three of the nine pods had started inside the hour, so their whole tree read as
changed. The churn row excludes them.

A repo with a packed `.git` compresses 1.4x, because the pack is already
compressed. A repo the agent has committed into holds up to 1,672 loose objects.

## What a full upload costs in time

Four steps run on every sync that clears the fingerprint gate. One of them is
instrumented.

| Step | Cost | Source |
|---|---|---|
| Fingerprint walk (`workspace_fingerprint`) | 0.14 s at 2,711 files | `measure` reports `walk_seconds` |
| Tar and gzip (`build_tar`) | 26 MB/s of raw tree on one core: 2 s at 52 MB, 6 s at the 160 MB fleet mean, 21 s at the 550 MB tail | `tar \| gzip -6` in a sandbox pod. `flate2` runs `rust_backend`, which does not beat zlib, so read this as the optimistic bound |
| Presign RPC and TLS | ~0.15 s. `http_client()` builds a new reqwest client per transfer, so every sync pays a 56 ms handshake | `curl` to the bucket from a pod |
| PUT | mean 0.85 s, worst observed 8.1 s, 98 MB/s | `moatless_workspace_sync_duration_seconds` |

`upload_from_config` starts its clock after `build_tar` returns, so the metric
reports the PUT alone — about a seventh of a mean-sized full upload.

Download reads mean 1.83 s, worst observed 62.2 s, and that clock covers the GET
and the extract together. It works out at 30.9 MB/s against the upload's 98 MB/s,
so the untar dominates the restore.

**The 10-second shutdown flush is the case this breaks.**
`SHUTDOWN_FLUSH_TIMEOUT` wraps the whole of `upload()`, so the walk and the
compression pass spend the budget before the PUT starts. Loki over 7 days counts
38 `Shutdown: workspace flush timed out` against 1,367 flushes, and the
cancelled future never reaches `report_workspace_sync` — the loss appears in no
metric and in no `workspace_sync_failed` event. The orderly stop path is safe:
`GrpcWorkspaceSync::SYNC_TIMEOUT` gives the pre-stop push 2 minutes.

A mirror changes the shape of this. It runs no compression pass at all, and a
request to S3 from a pod costs ~20 ms on a reused connection (7 ms TCP, 56 ms
handshake once). At that rate:

| Case | Objects | Time |
|---|---|---|
| Delta sync, 12 changed files | 12 | under 0.5 s |
| First upload of a workspace, 16-way | 3,000 | ~3.8 s plus 1.7 s of bytes |
| First upload of a workspace, 32-way | 3,000 | ~1.9 s plus 1.7 s of bytes |
| Restore, 32-way | 3,000 | ~1.9 s, no untar |

The steady state is what moves: 17k syncs a month go from 2 – 20 s to under a
second, and the shutdown flush stops racing its own deadline. The first upload
lands near today's mean and well inside today's tail. A mirror must hold one
client open across objects — at one handshake per object the seed would cost
three times as long as the tar it replaces.

## What each layout costs per month

S3 Standard list prices in eu-north-1: $0.023/GB-month, $0.005 per 1,000 PUTs,
$0.0004 per 1,000 GETs. Ingress is free and same-region egress to EC2 is free,
so no layout pays for transfer. The fleet inputs are the table above; the file
count and the compression ratio come from the single-repo sample.

| Layout | Objects | Storage | Storage $ | PUT $ | GET $ | Total $ |
|---|---|---|---|---|---|---|
| One gzipped tar per task (today) | 6,974 | 568 GB | 13.06 | 0.08 | 0.00 | **13.14** |
| Per-file mirror, delta sync | 18.9M | 1.4 TB | 33.60 | 5.99 | 4.68 | **44.27** |
| Per-file mirror, full re-upload | 18.9M | 1.4 TB | 33.60 | 229.27 | 4.68 | **267.54** |

The multi-repo sample (6,315 files, 1.4x ratio) prices the delta mirror at
$41.62. Raise the churn assumption from 12 to 200 changed files per sync and it
reaches $60.17. Backfilling all 6,974 existing archives into the mirror costs
$95 – $220 once, and buys nothing a task does not ask for, so let each mirror
fill on the task's next upload instead.

Three facts decide this:

1. **Storage doubles and the total stays small.** The mirror stores what gzip
   was removing, which is 1.4x – 2.6x. That is +$20/month at this fleet size.
2. **Requests are free at this rate.** 564 uploads/day is 17k syncs a month. A
   delta sync of 12 files costs $0.00006. Even a full re-upload of every file on
   every sync only reaches $229/month, and delta sync is what makes the mirror
   worth building anyway.
3. **The wire gets cheaper, not dearer.** Today each sync ships the whole 87 MB
   archive because one byte moved. A delta mirror ships the changed files: the
   sampled pods changed 14 KB to a few MB per window. That removes most of the
   49 GB/day, and the whole-tree compression pass with it. The measuring script
   spent 4 – 7 s on that pass per workspace; `build_tar` uses `flate2` and is
   faster, but it reads and compresses the same tree every time.

## What the model does not price

- **Engineering.** The mirror needs a key layout, a per-object diff against the
  bucket, deletion of removed paths, and a restore that fetches thousands of
  objects rather than one. None of that is in the numbers above.
- **The delete path.** `crates/s3/src/lib.rs:269` deletes a task's archive by
  paginating `list_objects_v2` and deleting each key on its own. At 1 object per
  task that is one call; at 3,000 it needs `delete_objects` batching (1,000 keys
  per request) before the mirror lands.
- **Restore latency.** One 87 MB GET becomes ~3,000 GETs. At 20 ms each that is
  60 s serial and about 2 s at 32-way parallelism. Measure it before choosing.
- **Consistency.** A tar is one atomic object: it restores a whole workspace or
  none of it. A mirror interrupted mid-sync restores a mixed tree. Keeping the
  tar as the restore path and adding the mirror for reads avoids this, and
  doubles the storage line to about $47/month.
- **Two settings that would change the answer.** Do not put the mirror on
  Standard-IA or Glacier: both bill a 128 KB minimum per object, and 62% of the
  files are under 4 KB, so a 160 MB workspace would bill as 384 MB. A lifecycle
  transition also charges $0.01 per 1,000 objects, which is $190 – $440 for one
  sweep of the fleet. Check bucket versioning too: with versioning on, every
  overwrite keeps the old object, so 17k syncs a month accumulate instead of
  replacing. One tar per task hides that; 3,000 objects per task would not.
- **What is actually in the bucket.** The 568 GB figure assumes all 6,974 tasks
  retain an archive at the mean size. `SANDBOX_RETAIN_STORAGE_ON_CLOSE` defaults
  to true and the provisioning Terraform writes no lifecycle rule, so nothing
  expires them, but no one has counted. With credentials for
  `soap-fp-staging-moatless-uploads`, this settles it:
  `aws s3 ls --summarize --human-readable --recursive s3://soap-fp-staging-moatless-uploads/workspaces/`.
