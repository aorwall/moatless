# Resources and recovery

Use [ResourceConfig](../../../../crates/api-schemas/src/workspace.rs) for the API
shape. Workspace `resources.requests` and `resources.limits` configure sandbox
CPU/memory; server entries can override their own requests/limits. Repository git
records do not own these settings.

Kubernetes workspace persistence has two independent switches:

| pvcEnabled | s3Enabled | Recovery source |
|---|---|---|
| true | true | Live PVC, with object-store snapshots as fallback |
| true | false | Live PVC |
| false | true | Snapshot restored into replacement workspace storage |
| false | false | Ephemeral workspace; no snapshot recovery |

Omitted values inherit runtime defaults. A snapshot is only as recent as its last
successful upload. `storage` sets capacity; `dockerStorage` and `dockerStorageMode`
are Docker-specific controls. Read [storage.rs](../../../../crates/sandbox-runtime/src/storage.rs)
for resolution and [workspace schema](../../../../crates/api-schemas/src/workspace.rs)
for the accepted fields.

An OOM termination and a disk-pressure eviction need different repairs. Before
large Rust builds, follow root disk checks and `task rust:gc`; do not share another
checkout's target directory. Capture the failed container's logs before redeploying.
