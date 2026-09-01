# moatless-sandbox-k8s (band 7)

The Kubernetes runtime provider behind the `moatless-sandbox-control` ports. It
implements `SandboxRuntime` (native mutation → `RuntimeOutcome`), the
`FleetObserver`, and the T3 read seams, and names `kube`/`k8s-openapi` freely —
those SDKs are legal in a provider band and invisible to the control band above,
which depends only on the ports. The backend composition root builds
`K8sSandboxAdapter` and wires it into `AppState`; this crate names no concrete
`AppState`. Its sibling `moatless-sandbox-docker` is the same band and is never
named from here.

## Load-Bearing Rules

- Provider adapters implement the control-band ports and return `RuntimeOutcome`;
  they do **not** publish lifecycle events (that is `Lifecycle`'s job in the
  control crate). Provision-plan construction and expected-image selection stay
  behind lifecycle and the concrete runtime adapters here.
- Native operations must prove observed absence before strict success and
  surface lingering resources (PVC included) as `RuntimeError::NotAbsent`.
- Kubernetes status reads direct from Deployments and Pods. Do not reintroduce
  Redis status caching.
- The pod watcher tracks pods by `pod_name`, not only `task_id`, so superseded
  generations are distinguishable from replacement crashes.
- Keep generated Kubernetes names and labels deterministic from `task_id`;
  preserve the 63-character Kubernetes cap in `naming.rs`.
- Server containers are data-driven from `ServerConfig`; do not hardcode server
  names or ports outside explicit config parsing.
- Every sandbox and server container carries CPU and memory requests + limits.
  `ResourceConfig` doubles as the storage config, so a per-repo override merges
  per key onto the `K8S_RESOURCE_*` defaults rather than replacing the block — a
  storage-only or memory-only override must not leave the container BestEffort.
- A runtime driver speaks the runtime contract (`moatless-sandbox-runtime`) and
  the domain floor (`moatless-types`), never the wire contract
  (`moatless-api-schemas`). A provisioning value the driver reads — a
  `RepoCheckout`, `WORKSPACE_REPOS_ENV` — is domain data and lives in
  `moatless_types::workspace`. `crate_deps.rs` fails the api-schemas edge.

## K8s Labels

- `moatless.io/task-id` identifies task-owned resources.
- `moatless.io/operation-generation` correlates runtime resources with lifecycle
  operations.
- `moatless.io/user-id` labels user secrets.
- `app.kubernetes.io/managed-by=moatless` identifies managed resources.
- `app.kubernetes.io/name=moatless-workspace`
- `app.kubernetes.io/component` is one of `sandbox`, `storage`, or
  `sandbox-service`.
