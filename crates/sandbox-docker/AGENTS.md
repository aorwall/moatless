# moatless-sandbox-docker (band 7)

The Docker Engine runtime provider behind the `moatless-sandbox-control` ports.
It manages a dedicated container per task on a shared Docker network (resolved by
container name as hostname for gRPC), implements `SandboxRuntime` (native
mutation → `RuntimeOutcome`), the `FleetObserver`, and the T3 read seams, and
names `bollard` freely — that SDK is legal in a provider band and invisible to
the control band above, which depends only on the ports. The backend composition
root builds `DockerSandboxAdapter` and wires it into `AppState`; this crate names
no concrete `AppState`. Its sibling `moatless-sandbox-k8s` is the same band and
is never named from here.

## Load-Bearing Rules

- Provider adapters implement the control-band ports and return `RuntimeOutcome`;
  they do **not** publish lifecycle events (that is `Lifecycle`'s job in the
  control crate). Provision-plan construction and expected-image selection stay
  behind lifecycle and the concrete runtime adapters here.
- Native operations must prove observed absence before strict success and
  surface lingering resources as `RuntimeError::NotAbsent`.
- Keep generated container names and labels deterministic from `task_id`
  (`naming.rs`), so a container/network is addressable without extra lookups.
- Server containers are data-driven from `ServerConfig`; do not hardcode server
  names or ports outside explicit config parsing.
- Every sandbox and server container carries CPU and memory requests + limits
  from `ResourceConfig`; a per-repo override merges per key onto the defaults
  rather than replacing the block.
- `classify_grpc_error()` (in `sandbox-proto`, reachable through the control
  crate's `client` module) must retain the underlying `tonic::Status` as the
  error source; the Docker connector surfaces gRPC failures through it rather
  than stringifying.
- A runtime driver speaks the runtime contract (`moatless-sandbox-runtime`) and
  the domain floor (`moatless-types`), never the wire contract
  (`moatless-api-schemas`). A provisioning value the driver reads — a
  `RepoCheckout`, `WORKSPACE_REPOS_ENV` — is domain data and lives in
  `moatless_types::workspace`. `crate_deps.rs` fails the api-schemas edge.
