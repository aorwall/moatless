# Split the pod's `Config` into a pod scope and a Member scope

A Sandbox can hold several Tasks. `sandbox-control` models that, and so does the
pod's `SandboxState::task_context`. `crates/sandbox/src/config.rs::Config` does
not.

## The three identities

| Identity | Scope | Owns |
|---|---|---|
| Sandbox | one pod | Deployment, PVC, workspace volume, credentials, preview proxy |
| Member | one Task in a pod | agent process, session, turns, command registry, checkout |
| Task | backend row | messages, turns, TTL, status |

A Sandbox id is the id of the Task that created it. So `task_id: String` names
two things, and each call site decides which one by convention.

`crates/sandbox-k8s/src/pod_builder.rs` writes `SANDBOX_ID` into the pod and
`Config::sandbox_id` reads it. No production code reads that field.

## What `Config` conflates

`Config` holds pod-scoped fields (`data_dir`, `runtime_dir`,
`claude_config_dir`, `servers`, `proxy`) and Member-scoped fields (`task_id`,
`workspace_dir`, `repo_dir`, `repos`) in one struct.

`task_context::registration::register` clones `Config` and overwrites three
fields per Member. `compose_task_context` passes that clone to `spawn_agent`,
then copies three of its strings into `TaskContext` and drops the rest. No
caller can reach a Member's `Config`.

So `state.config` means the boot Member, and nothing says so. The sandbox crate
reads `state.config.task_id` 63 times and `state.config.workspace_dir` 11 times.
29 of the 63 sit in `#[tracing::instrument]` attributes on gRPC handlers, so
every log line for a Member's RPC carries the boot Member's Task id.

`docs/deferred-changes.md` records the live defects that remain among the rest.

## The proposed seam

Store on `TaskContext` the `Arc<Config>` that `compose_task_context` already
builds, and delete the three strings `TaskContext` copies out of it.
`TaskContext::new` loses three arguments, and it carries
`#[allow(clippy::too_many_arguments)]` today.

Then remove `task_id`, `workspace_dir`, `repo_dir` and `repos` from the
pod-level `Config`, and let `Config::sandbox_id` name the pod. The compiler
reports every read site, and each one has to state which scope it means.

Change the 29 tracing attributes to log the resolved Member.

Add no trait. One implementation means one concrete type.

## Not covered here

`SyncWorkspace` and the eight terminal RPCs carry no `task_id`. `SyncWorkspace`
is correctly pod-scoped. The terminal RPCs resolve their Task in
`moatless_t3code::terminal`, and this document did not check that path.
