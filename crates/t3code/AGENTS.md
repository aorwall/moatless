# crates/t3code

## Writing a projection

A read projection may join domain tables to preserve the T3 response shape.
[Table ownership](../../tooling/arch-rules/src/table_ownership.rs) checks
writes separately, and the architecture baseline holds the existing violations.

Two rules bound that licence.

**Every statement here is a read.** Issue no `INSERT`, `UPDATE` or `DELETE`
against a table another crate owns; ask through a port, such as
`ThreadCommandSink` or `SandboxStarter`, or do not make the write. When one of
these reads has to become a write, the exclusion stops applying and the read
model routes through the ports instead. `docs/deferred-changes.md` records that
entry condition.

**A read whose shape another band already publishes goes through the seam.**
Activities are the worked example. `ActivitySource` in
`moatless_ports::activity` serves the four reads the Task band already had, and
this crate names the port rather than the crate. Reserve raw SQL here for the
joins only a T3 transcript asks for.

## Sharing the activity vocabulary

`Activity`, `ActivityKind`, `ActivityStatus`, `ActivityView` and the tool
constants live in `moatless_types::activity`; `moatless_ports::activity` holds
`ActivitySource` alone. The band that writes an activity and the projection that
renders it read one definition.

Do not re-declare a T3-shaped copy of a Moatless row here. The translation
belongs under `projection/`, in the child named for the wire shape it builds.

## Reaching another band through a seam

The seams are how this crate reaches anything it does not read directly.
`AgentCatalogSource`, `RepositoryBranchSource`, `BoundPullRequestSource`,
`SourceControlAuthSource`, `TaskRelationSource` and `ThreadCommandSink` are
declared here and implemented over `AppState` in
`crates/server/src/t3code/backend.rs`, beside each other.

`TaskRelationSource` is a seam and not a sixteenth table: `task_relations`
belongs to the Task band. The seam answers with every edge and decides no
access, and `ConversationDao::readable_by_ids` re-reads the children it names,
so one rule settles what a viewer sees of a Task tree.

`ActivitySource` is the exception: it is declared in `moatless_ports::activity`
and implemented by the Task band itself (`moatless_tasks::ports::Activities`).
Add a seam. Do not add a dependency on the crate behind it.

`moatless-tasks` and `moatless-workspaces` are `[dev-dependencies]` here, for the
row factories the `#[sqlx::test]` blocks in `dao.rs` seed with. Keep them there:
under `[dependencies]` they are the band edge this section forbids.

## Answering a request off the frame loop

One connection reads one frame at a time, so a request answered inside the loop
holds every frame the client sent after it. `rpc::dispatch::routing::detach` is
the whole list of the methods that do not: it spawns them onto the connection's
frame channel and the loop reads on.

**Detach a read; keep a transition on the loop.** A method that moves a named
resource from one state to another — `terminal.*`, `projects.writeFile`,
`sandbox.start`/`stop`, `scripts.run`, `orchestration.dispatchCommand`,
`attachments.delete`, `git.runStackedAction`, the mutating `preview.*` — stays
in `handle_request`. The loop is sequential, so a transition on it finishes
before anything the client sent after it is dispatched; that is what a detached
read is allowed to rely on, and detaching a transition would take it away.

**A detached method may not touch `ProtocolSession`.** Each one is a free
function taking an owned `Request` for that reason — the compiler enforces it.
State a detached method genuinely needs goes in `SharedSession`
(`rpc/session.rs`) behind its own lock, held for a single read or write and
never across an await.

**One function per method, under `rpc/dispatch/`, grouped by the surface it
answers for.** `detach` and `handle_request` are routing tables and hold no
method body; both live in `dispatch/routing.rs`, and the two matches together are
the whole list of what this surface serves. Name a method's function for its tag,
snake cased: `pullRequests.summary` is `pull_requests_summary`. A method that
holds the frame loop goes in `dispatch/streams.rs`.

The subscription-open arms are on the loop and hold it; see
`docs/deferred-changes.md`.

## Measuring the RPC surface

`rpc::metrics::answered` counts, times and marks every request, on the frame loop
and off it, reading the outcome off the frames the handler returned. It is the
only caller of `record_request`, so `duration_ms` measures the handler wherever it
ran. A dispatch arm builds its own frames and reports no outcome of its own.

**Do not add a counter to a dispatch arm.** Add whatever the outcome needs to
`rpc::metrics::Outcome` and let the seam emit it; two counters drift.

`rpc::metrics::CONNECTION_BLOCKED` is the one metric emitted outside that seam,
from `rpc/socket.rs::serve_websocket`, because it measures the connection and
not the request: a detached request contributes the dispatch that spawned it and
not the handler that follows. A metric whose subject is the connection goes in
the connection loop; everything a request answers with goes through `answered`.

**The `tag` label is bounded by outcome, and never by trust.** A tag is a
string the client chose, so only an outcome a matched dispatch arm could have
produced carries the raw tag. Two collapse to a constant in
`Outcome::tag_label`: `Unsupported`, the `_ =>` catch-all any string reaches,
and `Rejected`, the pre-dispatch id checks that run before anything looks at the
tag. Add each new outcome reachable without a matched arm to it.

`connection_defect` takes a fixed `reason` word for the same reason; its message
carries a serde error or a request id.

**Deployments run at `RUST_LOG=info`.** Nobody will ever read a refusal logged
at `debug!`. Log every decision this surface makes about a client's request at
`warn!`.

## Renaming a thread

A Task created from T3 is created unnamed. `rename` in `commands.rs` drops a
`thread.meta.update` title unless the Task already has a name. The client's
auto-title and a person's rename are identical on the wire, and only the
auto-title arrives while the Task is still unnamed; letting it through sets
`tasks.name` before the pod boots and suppresses the `# Required First Actions`
block that has the agent name the Task (`crates/tasks/src/agent/prompt.rs`).

Known gap: a thread created with no first message never runs an agent, so it
stays unnamed, cannot be renamed, and shows the contract's "New thread"
placeholder.
