# S3 Workspace Sync (alternative to PVC storage)

**Status:** Proposed
**Domain prefix:** `SANDBOX`
**Related:** [00-design.md](./00-design.md), [01-sandbox-scenarios.md](./01-sandbox-scenarios.md)

## Summary

Today every task's sandbox mounts a per-task **EBS PVC** (`workspace-data`, `ReadWriteOnce`) at
`/opt/moatless` to persist the git checkout and agent edits across stop/start. The PVC path has
real costs: `ReadWriteOnce` forces `strategy: Recreate` plus a `wait_for_pods_deleted` drain on
every restart, EBS attach/detach is slow, and each task carries a standing EBS volume.

This spec adds an alternative storage backend selectable **per repository**: instead of a PVC, the
sandbox uses a node-local `emptyDir` workspace and **syncs the working tree to S3**. Both backends
run side by side, gated by `resources.storage_mode`, and are instrumented with a `storage_backend`
label so we can compare startup latency, restart latency, sync cost, and reliability before
considering a default change.

## Background: what exists today

| Piece | Location | Note |
|---|---|---|
| Per-repo storage toggle | `crates/api-schemas/src/repository.rs` — `ResourceConfig.storage_mode` | `"ebs"` (default, PVC) / `"ephemeral"` (emptyDir, no persistence). Stored in the `resources` jsonb column — **no migration needed to add a value**. |
| PVC vs emptyDir routing | `backend/src/sandbox/k8s/provider.rs` — `is_ephemeral_storage()`, `create_or_find_pvc()` | Branches on `storage_mode`. |
| Existing S3 sync (template) | `sandbox/src/init/transcript_sync.rs` + `backend/src/sandbox/grpc_server.rs` (`UploadTranscript`/`DownloadTranscript`) | Tars `.claude/projects` and ships it to S3 **through the backend**; sandbox is AWS-credential-less. |
| S3 primitives | `backend/src/file_storage/s3.rs` — `put_object`, `get_object_opt`, `delete_objects_by_prefix` | Single `put_object`, no multipart; buffers full object in memory. |
| Metrics pattern | `backend/src/*/metrics.rs`, registered in `startup.rs::spawn_metrics_server()` | `moatless_<domain>_<noun>_<unit>`; sandbox emits none directly — sends timing via `PublishEvent`. |

## The transport decision: why presigned URLs, not the gRPC proxy

The transcript proxy works because a transcript tar is a few MB. A **workspace** can be hundreds of
MB up to the 3Gi limit, and the proxy path does not scale to that:

- **Double in-memory buffering.** Upload reassembles all 1 MiB chunks into one `Vec<u8>` before
  `put_object`; download buffers S3 → backend `Vec<u8>` → chunks → sandbox `Vec<u8>`. The whole tar
  sits in backend heap *and* sandbox heap at once.
- **Hard 10 s per-request gRPC timeout** (`sandbox/src/backend_client.rs:135`) severs any tar that
  takes longer than ~10 s to stream (~tens of MB).
- **No multipart**; single `put_object(ByteStream::from(vec))`.
- **Backend is a shared chokepoint** — every task's sync funnels through one process's memory and NIC.

Giving the sandbox a **standing IAM identity (IRSA)** would fix throughput but breaks the
credential-less design that is stated as intentional in `grpc_server.rs:788`,
`transcript_sync.rs:7`, and the helm README — agent-executed code would run with cloud credentials.

**Chosen approach — presigned URLs.** The backend (which already holds the S3 client and IAM
identity) mints a short-lived, single-object, single-verb **presigned URL** and hands it to the
sandbox over the existing JWT gRPC channel. The sandbox then does a plain HTTPS `PUT`/`GET`
**directly to S3**.

```
sandbox --(JWT gRPC: GetWorkspaceSyncUrls)--> backend
backend --(presign PUT+GET, key=<workspace_prefix>/<task_id>/workspace.tar, TTL=15m)--> sandbox
sandbox --(HTTPS PUT/GET tar bytes)----------> S3      # direct; backend not in the data path
```

Why this wins:
- **Sandbox stays AWS-credential-less** — it only ever holds a URL that expires and is scoped to one
  object + one verb. Blast radius ≈ one task's tar for 15 minutes.
- **Direct data path** — no backend buffering/bandwidth, no 10 s gRPC ceiling, streaming + optional
  multipart via a plain HTTP client.
- **Backend keeps enforcement** — it validates the JWT's `task_id` and only ever presigns that
  task's key, exactly as `transcript_key()` derives keys today.
- **Small delta** — reuse `file_storage/s3.rs` (add presign helpers), add **one** RPC
  (`GetWorkspaceSyncUrls`) instead of two streaming ones, and a `reqwest`-based `workspace_sync`
  module in the sandbox.

Constraints: S3 single-`PUT` cap is 5 GB (> 3Gi limit, so single-part is enough for MVP; multipart
only if the cap rises). Sandbox needs S3 `:443` egress — allowed under default `broadEgress:true`;
hardened `broadEgress:false` clusters need an S3 FQDN/CIDR rule (document it). TTL is re-minted each
sync tick, so expiry during a long pod life is a non-issue.

## Sync scope

Sync **only** the git working tree under `/opt/moatless/repo`: tracked + untracked-non-ignored files
(`git ls-files -c -o --exclude-standard`). This captures source + agent edits and **excludes**
`node_modules`, `dist`, `target`, caches — which are regenerated by setup commands and the existing
NVMe `pkg-cache` hostPath volumes. Transcripts (`.claude/projects`) and `codex/auth.json` already
have their own sync loops and are left unchanged.

S3-mode repos should be paired with `pkg-cache` so `node_modules` isn't refetched every boot
(emptyDir loses it). Document this in the repo-config guidance.

## Sync lifecycle (sandbox side)

Modeled on `transcript_sync.rs`:
- **download-on-start** — during Phase-1 init, *after* `git clone`, *before* `Ready`: fetch and
  overlay the S3 tar (agent edits/uncommitted changes) on top of the fresh clone. Clone still skips
  if `.git` exists.
- **periodic** — tokio loop, `WORKSPACE_SYNC_INTERVAL_SECS` (default 60), re-mints presigned URLs.
- **after-turn** — trigger a sync when the agent goes idle, bounding data-loss-on-eviction to the
  in-flight turn only.
- **flush-on-shutdown** — on SIGTERM, flush within the 30 s drain grace (bounded timeout like the
  transcript flush's 12 s).
- **consistency** — build the tar while the agent is idle (after-turn) or from a `git stash create`
  snapshot to avoid mid-write tears.

## Metrics (comparison is a first-class goal)

Add a `storage_backend` dimension (`ebs` / `ephemeral` / `s3`) so the PVC path is the measurable
baseline. All `moatless_<domain>_<noun>_<unit>`, registered in `spawn_metrics_server()`.

| Metric | Type | Labels | Captures |
|---|---|---|---|
| `moatless_sandbox_provision_duration_seconds` | histogram | `storage_backend` | provision call time |
| `moatless_sandbox_ready_duration_seconds` | histogram | `storage_backend` | provision → health `Ready` (EBS attach vs S3 download) |
| `moatless_sandbox_restart_duration_seconds` | histogram | `storage_backend` | RWO detach-wait for EBS vs none for S3 |
| `moatless_workspace_sync_duration_seconds` | histogram | `direction` | one sync (up/down) |
| `moatless_workspace_sync_bytes` | histogram | `direction`,`storage_backend` | tar size distribution |
| `moatless_workspace_sync_total` | counter | `direction`,`outcome` | success/failure counts |

Sandbox emits sync timing/bytes via `PublishEvent`; the backend converts to Prometheus in the
event-handler metrics path (same as agent turn metrics). Provision/ready/restart are timed
backend-side in `provider.rs`/`deployment.rs`. Ship a Grafana panel comparing ready/restart/EBS-cost
by `storage_backend` plus a sync-size/failure panel.

## Implementation phases

- **Phase 0 — Comparison metrics.** Add `storage_backend` label + provision/ready/restart
  histograms. Low-risk, benefits the existing `ebs` path immediately.
- **Phase 1 — Config + pod routing.** Recognize `storage_mode: "s3"`; route to emptyDir workspace
  (reuse ephemeral path); inject `WORKSPACE_SYNC_ENABLED` / `WORKSPACE_SYNC_INTERVAL_SECS` env.
- **Phase 2 — Presign transport.** `WORKSPACE_S3_PREFIX` config (default `"workspaces"`, same
  bucket); `presign_put`/`presign_get` in `file_storage/s3.rs`; `GetWorkspaceSyncUrls` gRPC RPC
  (JWT-`task_id`-scoped); delete workspace archive on `cleanup()` alongside the transcript delete.
- **Phase 3 — Sandbox `workspace_sync` module.** download-on-start / periodic / after-turn /
  flush-on-shutdown using `reqwest` against the presigned URLs; `git ls-files`-scoped tar.
- **Phase 4 — Sync metrics + dashboard.**
- **Phase 5 — Rollout.** Default stays `ebs`; opt a few repos into `s3`; run both ~1 week; compare;
  then decide on default.

## Risks & decisions

1. **Data-loss window.** SIGKILL (eviction/OOM) between syncs loses edits since the last sync — the
   PVC has none. Mitigated by after-turn sync + shutdown flush + short interval. Acceptable for an
   opt-in mode; PVC stays the safe default.
2. **`node_modules` regeneration.** emptyDir loses it each boot → S3 repos should use `pkg-cache`.
3. **Hardened network policy.** `broadEgress:false` clusters need an S3 egress rule for the direct
   HTTPS PUT/GET.

---

## Scenarios

New IDs start at `SANDBOX-20` (existing sandbox scenarios end at `SANDBOX-07`).

### SANDBOX-20: S3-backed sandbox provisions without a PVC

A repo configured with `storage_mode: "s3"` gets a node-local workspace and no EBS volume.

* **Given** a repository with `resources.storage_mode` set to `"s3"`

* **When** a task is created and its sandbox is provisioned

* **Then** no PVC is created for the task (the `workspace-data` volume is an `emptyDir`), the pod
  reaches health `Ready`, and the sandbox container has `WORKSPACE_SYNC_ENABLED=true` in its env

### SANDBOX-21: Agent edits survive a stop/start via S3

Working-tree changes are restored from S3 on resume even though the node-local workspace is gone.

* **Given** SANDBOX-20 — an `s3`-mode sandbox with a completed agent turn that modified a tracked file

* **When** the sandbox is stopped (pod deleted, `emptyDir` discarded) and then started again

* **Then** the workspace is re-cloned fresh, the S3 workspace archive is downloaded before `Ready`,
  and the previously modified file contains the agent's changes (not the pristine clone contents)

### SANDBOX-21.1: Untracked files are synced, ignored files are not

Sync scope matches `git ls-files -c -o --exclude-standard`.

* **Given** SANDBOX-21 — an `s3`-mode sandbox

* **When** an untracked non-ignored file and a `.gitignore`d path (e.g. `node_modules/x`) both exist
  before a sync, and the sandbox is stopped and started

* **Then** after resume the untracked non-ignored file is present and the ignored path is absent

### SANDBOX-22: Shutdown flush persists the latest edits

A graceful stop captures edits made since the last periodic sync.

* **Given** SANDBOX-20 — an `s3`-mode sandbox with an idle agent

* **When** a file is written and the sandbox receives SIGTERM before the next periodic sync tick

* **Then** the shutdown flush uploads the workspace within the drain grace period, and starting the
  sandbox again restores that file

### SANDBOX-23: Cleanup removes the S3 workspace archive

No orphaned S3 objects after a task is cleaned up.

* **Given** SANDBOX-20 — an `s3`-mode sandbox that has synced at least once

* **When** the task is cleaned up (auto-close or explicit cleanup)

* **Then** the `<workspace_prefix>/<task_id>/` S3 objects are deleted alongside the transcript archive

### SANDBOX-24: Storage backend is observable via metrics

Both backends emit comparable timing metrics tagged by backend.

* **Given** one `ebs`-mode and one `s3`-mode sandbox are provisioned

* **When** each reaches `Ready` and performs at least one workspace sync (s3) / attach (ebs)

* **Then** `moatless_sandbox_ready_duration_seconds` carries a `storage_backend` label for both, and
  `moatless_workspace_sync_total{outcome="ok"}` is non-zero for the `s3` task
