# Resources and recovery

Use [ResourceConfig](../../../../crates/workspace-types/src/schemas/workspace.rs) for the API
shape. Workspace `resources.requests` and `resources.limits` configure sandbox
CPU/memory. Repository git records do not own these settings.

Kubernetes workspace persistence has two independent switches:

| pvcEnabled | s3Enabled | Recovery source |
|---|---|---|
| true | true | Live PVC, with object-store snapshots as fallback |
| true | false | Live PVC |
| false | true | Snapshot restored into a new pod; an in-place container restart keeps its `emptyDir` |
| false | false | Ephemeral workspace; no snapshot recovery |

Omitted values inherit runtime defaults. A snapshot is only as recent as its last
successful upload. `storage` sets capacity; `dockerStorage` and `dockerStorageMode`
are Docker-specific controls. Read [storage.rs](../../../../crates/sandbox-runtime/src/storage.rs)
for resolution and [workspace schema](../../../../crates/workspace-types/src/schemas/workspace.rs)
for the accepted fields.

An OOM termination and a disk-pressure eviction need different repairs. Before
large Rust builds, follow root disk checks and `task rust:gc`; do not share another
checkout's target directory. Capture the failed container's logs before redeploying.

A `container_restarted` event with `exit 137` and reason `Error`, not
`OOMKilled`, is the kubelet killing the container in place, most often for a
failed `/livez` probe. When several sandboxes on one node restart together, read
that node's `node_memory_MemAvailable_bytes` and the gaps in its node-exporter
scrapes: sandbox memory limits far above the node's allocatable let the node
run out before any one sandbox reaches its own limit.
