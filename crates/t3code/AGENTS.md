# crates/t3code

## Writing a projection

A read projection may join domain tables to preserve the T3 response shape.
[Table ownership](../../tooling/arch-rules/src/table_ownership.rs) checks
writes separately, and the architecture baseline holds the existing violations.

Two rules bound that licence.

**Every statement here is a read.** Issue no `INSERT`, `UPDATE` or `DELETE`
against a table another crate owns. A write is that owner's to make. Ask for it
through a port, such as `ThreadCommandSink` or `SandboxStarter`, or do not make
it. When one of these reads has to become a write, the exclusion stops applying
and the read model has to route through the ports instead.
`docs/deferred-changes.md` records that entry condition.

**A read whose shape another band already publishes goes through the seam.**
Activities are the worked example. `ActivitySource` in
`moatless_ports::activity` serves the four reads the Task band already had, and
this crate names the port rather than the crate. Reserve raw SQL here for the
joins only a T3 transcript asks for.

## Sharing the activity vocabulary

`Activity`, `ActivityKind`, `ActivityStatus`, `ActivityView` and the tool
constants live in `moatless_ports::activity`, beside `ActivitySource`. The band
that writes an activity and the projection that renders it read one definition.

Do not re-declare a T3-shaped copy of a Moatless row here. The translation
belongs in `projection.rs`, where the wire shape is built.

## Reaching another band through a seam

The seams are how this crate reaches anything it does not read directly.
`AgentCatalogSource`, `RepositoryBranchSource`, `BoundPullRequestSource`,
`SourceControlAuthSource`, `TaskRelationSource` and `ThreadCommandSink` are
declared here and implemented over `AppState` in
`crates/server/src/t3code/backend.rs`, beside each other.

`TaskRelationSource` is a seam and not a sixteenth table, for a stated reason.
`task_relations` is the Task band's table and `TaskRelationDao` already
publishes the read. The seam answers with every edge and decides no access.
`ConversationDao::readable_by_ids` re-reads the children it names. One rule
therefore settles what a viewer sees of a Task tree and what a link shows.

`ActivitySource` is the exception. It is declared in `moatless_ports::activity`
and implemented by the Task band itself (`moatless_tasks::ports::Activities`),
because the reads it serves are the Task band's own. Add a seam. Do not add a
dependency on the crate behind it.

## Measuring the RPC surface

`record_request` (`rpc/metrics.rs`) counts, times and marks every request. The one
`ClientFrame::Request` arm calls it. It reads the outcome off the frames the
handler returned. The fifty dispatch arms each build their own frames, and none
of them reports an outcome of its own.

**Do not add a counter to a dispatch arm.** Add whatever the outcome needs to
`rpc::metrics::Outcome` and let the seam emit it. Two counters drift, and a
reader then cannot tell which one is the surface's.

`rpc::metrics::CONNECTION_BLOCKED` is the one metric emitted outside that seam,
from `rpc/socket.rs::serve_websocket`. It measures the connection and not the
request: a frame is already read by the time `incoming.next()` yields it, so the
wait a queued request suffers is not observable from the seam that answers it. A
metric whose subject is the connection goes in the connection loop; everything
a request answers with still goes through `record_request`.

**The `tag` label is bounded by outcome, and never by trust.** A tag is a
string the client chose, so labelling with it directly gives an unbounded label
set. Only an outcome that a matched dispatch arm could have produced carries
the raw tag. Two outcomes collapse to a constant. `Unsupported` is the `_ =>`
catch-all, which any string at all reaches. `Rejected` covers the pre-dispatch
id checks, which run before anything looks at the tag. `Outcome::tag_label` is
where that holds. Add each new outcome reachable without a matched arm to it.

`connection_defect` takes a fixed `reason` word for the same reason. Its
message carries a serde error or a request id.

**Deployments run at `RUST_LOG=info`.** Nobody will ever read a refusal logged
at `debug!`. Log every decision this surface makes about a client's request at
`warn!`.

## Renaming a thread

A Task created from T3 is created unnamed. `rename` in `commands.rs` passes on
a `thread.meta.update` that carries a title only while the Task already has a
name.

The client titles a new thread from its first message within seconds of
creating it. That title reaches `tasks.name` before the pod boots, which
suppresses the `# Required First Actions` block that has the agent name the
Task (`crates/tasks/src/agent/prompt.rs`). Both writes look identical on the
wire. The auto-title is separated from a person's rename by one fact: the
auto-title lands while the Task is still unnamed.

This leaves one gap. A thread created with no first message never runs an
agent, so it stays unnamed and T3 cannot rename it either. It shows the
contract's "New thread" placeholder.
