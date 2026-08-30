# crates/t3code

The T3 compatibility surface: discovery, session bootstrap,
`/api/orchestration/shell`, and the `/ws` Effect-RPC envelopes, served from
Moatless rows. An anti-corruption layer — T3 names appear on the wire, Moatless
names everywhere behind it.

## This crate is a read-only projection

It reads fifteen tables it does not own — `tasks`, `task_messages`,
`task_turns`, `task_activities`, `task_follows`, `task_bindings`,
`task_message_usage`, `task_message_contexts`, `workspaces`, `workspace_repos`,
`users`, and the four listing/conversation change-feed tables. That is the
declaration, and it is what `scripts/check-table-ownership.sh` excludes this
crate on: a projection over another band's tables reads across them by
construction, and routing every read through a port would put the projection's
shape in the bands it projects.

**Two rules follow, and they are the whole of the licence.**

- **Every statement here is a read.** No `INSERT`, `UPDATE` or `DELETE` against
  a table another crate owns. A write is that owner's to make; this crate asks
  for it through a port (`ThreadCommandSink`, `SandboxStarter`) or does not make
  it. The moment one of these reads has to become a write, the exclusion stops
  applying and the read model has to be routed through the ports instead — that
  is the entry condition recorded in `docs/deferred-changes.md`.
- **A read whose shape another band already publishes goes through the seam.**
  Activities are the worked example: `ActivitySource` in
  `moatless_backend_core::activity` serves the four reads the Task band already
  had, and this crate names it rather than that crate. Raw SQL here is for the
  joins only a T3 transcript asks for.

## The vocabulary is shared, not copied

`Activity`, `ActivityKind`, `ActivityStatus`, `ActivityView` and the tool
constants live in `moatless_backend_core::activity`, beside `ActivitySource`.
Both the band that writes an activity and the projection that renders it read
one definition. Do not re-declare a T3-shaped copy of a Moatless row here — the
translation belongs in `projection.rs`, where the wire shape is built.

## Ports over sources

The seams are how this crate reaches anything it does not read directly. Four
(`AgentCatalogSource`, `RepositoryBranchSource`, `BoundPullRequestSource`,
`SourceControlAuthSource`) and `ThreadCommandSink` are declared here and
implemented over `AppState` in `crates/backend/src/t3code/backend.rs`, beside
each other. `ActivitySource` is the exception: it is declared in
`moatless_backend_core::activity` and implemented by the Task band itself
(`moatless_tasks::ports::Activities`), because the reads it serves are the Task
band's own and existed before this crate wanted them. Add a seam, not a
dependency on the crate behind it.
