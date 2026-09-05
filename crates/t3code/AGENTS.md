# crates/t3code

The T3 compatibility surface: discovery, session bootstrap,
`/api/orchestration/shell`, and the `/ws` Effect-RPC envelopes, served from
Moatless rows. An anti-corruption layer — T3 names appear on the wire, Moatless
names everywhere behind it.

## Projection ownership

Read projections may join domain tables to preserve the T3 response shape.
[Table ownership](../../tooling/arch-rules/src/table_ownership.rs) checks writes
separately. Existing violations are in the architecture baseline.

**Two rules follow, and they are the whole of the licence.**

- **Every statement here is a read.** No `INSERT`, `UPDATE` or `DELETE` against
  a table another crate owns. A write is that owner's to make; this crate asks
  for it through a port (`ThreadCommandSink`, `SandboxStarter`) or does not make
  it. The moment one of these reads has to become a write, the exclusion stops
  applying and the read model has to be routed through the ports instead — that
  is the entry condition recorded in `docs/deferred-changes.md`.
- **A read whose shape another band already publishes goes through the seam.**
  Activities are the worked example: `ActivitySource` in
  `moatless_ports::activity` serves the four reads the Task band already
  had, and this crate names it rather than that crate. Raw SQL here is for the
  joins only a T3 transcript asks for.

## The vocabulary is shared, not copied

`Activity`, `ActivityKind`, `ActivityStatus`, `ActivityView` and the tool
constants live in `moatless_ports::activity`, beside `ActivitySource`.
Both the band that writes an activity and the projection that renders it read
one definition. Do not re-declare a T3-shaped copy of a Moatless row here — the
translation belongs in `projection.rs`, where the wire shape is built.

## Ports over sources

The seams are how this crate reaches anything it does not read directly. Five
(`AgentCatalogSource`, `RepositoryBranchSource`, `BoundPullRequestSource`,
`SourceControlAuthSource`, `TaskRelationSource`) and `ThreadCommandSink` are
declared here and implemented over `AppState` in
`crates/server/src/t3code/backend.rs`, beside each other.

`TaskRelationSource` is a seam rather than a sixteenth table for the stated
reason: `task_relations` is the Task band's and `TaskRelationDao` already
publishes the read. It answers with every edge and decides no access — the
children it names are re-read through `ConversationDao::readable_by_ids`, so
what a viewer is shown of a Task tree is settled by the same rule a link is.

`ActivitySource` is the exception: it is declared in
`moatless_ports::activity` and implemented by the Task band itself
(`moatless_tasks::ports::Activities`), because the reads it serves are the Task
band's own and existed before this crate wanted them. Add a seam, not a
dependency on the crate behind it.

## One seam measures the RPC surface

Every request is counted, timed and marked in `record_request` (`lib.rs`),
called from the one `ClientFrame::Request` arm. It reads the outcome off the
frames the handler returned, because the fifty dispatch arms each build their
own frames and none of them reports an outcome of its own. **Do not add a
counter to a dispatch arm** — add whatever the outcome needs to
`rpc_metrics::Outcome` and let the seam emit it, or the two counts drift and a
reader cannot tell which is the surface's.

**The `tag` label is bounded by outcome, not by trust.** A tag is a string the
client chose, so labelling with it directly is an unbounded label set. Only an
outcome a matched dispatch arm could have produced carries the raw tag;
`Unsupported` (the `_ =>` catch-all, reachable for any string at all) and
`Rejected` (the pre-dispatch id checks, which run before the tag is looked at)
collapse to a constant. `Outcome::tag_label` is where that holds, and a new
outcome reachable without a matched arm has to be added to it.

For the same reason `connection_defect` takes a fixed `reason` word rather than
labelling with its message: the message carries a serde error or a request id.

**Deployments run at `RUST_LOG=info`.** A refusal logged at `debug!` is a
refusal nobody will ever read. Log a decision this surface makes about a
client's request at `warn!`.

## The client renames a thread, it does not name one

A Task created from T3 is created unnamed, and a `thread.meta.update` carrying a
title is passed on only when the Task already has a name (`rename` in
`commands.rs`). The client titles a new thread from its first message within
seconds of creating it, and that title reaches `tasks.name` before the pod
boots, which suppresses the `# Required First Actions` block that has the agent
name the Task (`crates/tasks/src/agent/prompt.rs`). Both writes look identical
on the wire — the only thing separating the auto-title from a person's rename is
that the auto-title lands while the Task is still unnamed.

The corner this leaves: a thread created with no first message never runs an
agent, so it stays unnamed and cannot be renamed from T3 either. It shows the
contract's "New thread" placeholder.
