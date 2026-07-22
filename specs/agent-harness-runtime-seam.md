# Spec: Deep Agent harness seam

> Status: ready-for-agent
> Domain vocabulary: `CONTEXT.md`
> Architectural decisions: ADR-0002, ADR-0003, ADR-0004, ADR-0005

## Problem Statement

Moatless Vibe runs Claude Code and Codex inside a Sandbox, but the current Agent
execution architecture makes each harness a cross-cutting concern. The central
actor, Session startup, retry handling, event translation, cancellation, and
tests all branch on harness-specific behavior. Adding another Agent harness
requires coordinated edits across the central lifecycle implementation instead
of adding one implementation behind a stable seam.

This causes three user-visible risks. Agent behavior can diverge between
harnesses, lifecycle bugs in one integration can affect the other, and changes
take longer because maintainers must understand a large actor with mixed Task,
Turn, Run, Session, process, transport, and publishing responsibilities. The
current implementation also contradicts the accepted server-owned Message
assembly decisions: Claude content blocks are coalesced in the Sandbox even
though the backend is intended to be the only Message assembler.

The implementation has become difficult to navigate. Important files have grown
to roughly one thousand lines, and an abandoned alternative executor remains in
the tree even though it is not compiled. A future Agent harness should not make
the central actor longer or introduce another parallel execution architecture.

## Solution

Introduce a deep **Agent harness** module with one small command/signal interface.
A Task selects one harness for its entire lifetime. Each Sandbox creates one
instance of that harness, which controls at most one active Agent process while
allowing the Session to survive process replacement and Sandbox recreation.

The Agent harness owns all behavior that requires knowledge of Claude Code,
Codex, or a future CLI: Session start and resume, input delivery, persistent
versus one-shot process strategy, native output parsing, cancellation, stable
native identity, and harness-error classification. A harness-neutral coordinator
owns Task, Turn, and Run state, pending-Message acknowledgement, retry policy,
and backend event publishing.

The interface consists of input submission, cancellation, and an ordered stream
of normalized signals. The coordinator consumes the same signals whether Claude
accepts input into a persistent process, Codex queues it for a replacement
process, or a persistent Agent process starts work internally.

Each native content block becomes one canonical source event. The backend groups
those blocks by native Message identity, admits repeated usage once, persists the
canonical Message rows, resolves the affected parent, and emits derived UI
upserts. Sandbox-side Message coalescing is removed.

The implementation is organized into focused private modules. File size is a
review signal, not an abstraction target: production files approaching roughly
200–300 lines should be checked for multiple responsibilities, but code is split
only at meaningful seams or around owned concepts.

## User Stories

1. As a Task creator, I want my selected Agent harness to remain fixed for the lifetime of the Task, so that its Session and behavior remain coherent.
2. As a Task creator, I want an existing Task to continue with the same harness after a Sandbox restart, so that infrastructure recovery does not change the Agent unexpectedly.
3. As a Task creator, I want `start_fresh` to begin a new Session without changing the harness, so that I can reset conversation context predictably.
4. As a Task creator, I want a resumed Task to keep its CLI Session when the Agent process is replaced, so that prior conversation context remains available.
5. As a Task creator, I want process replacement to be invisible unless Session recovery fails, so that ordinary CLI lifecycle details do not interrupt my work.
6. As a Task creator, I want a clear context-loss notice when a Session cannot be resumed, so that I understand why the Agent may need to recover history.
7. As a Task creator, I want cancellation to stop the active Agent process and close the current Turn exactly once, so that stopping work is reliable.
8. As a Task creator, I want repeated cancellation requests to be safe, so that retries or duplicate requests cannot corrupt lifecycle state.
9. As a Task creator, I want authentication and configuration failures to surface consistently across harnesses, so that failures are understandable.
10. As a Task creator, I want transient harness failures to follow one retry policy, so that recovery is consistent regardless of the selected CLI.
11. As a Task creator, I want a rejected input to remain unacknowledged until it is actually accepted or terminally failed, so that Messages are not lost.
12. As a Task creator, I want accepted input to be associated with the correct Session and Run, so that conversation history is assigned to the correct Turn.
13. As a Task creator, I want input sent during an active compatible Claude execution to join the current Run, so that interactive follow-up behavior remains responsive.
14. As a Task creator, I want input sent during an active Codex execution to remain ordered and start a later Run, so that a one-shot process never receives unsupported injection.
15. As a Task creator, I want model, mode, or system-prompt changes to trigger whatever process replacement the harness requires, so that configuration changes take effect safely.
16. As a Task creator, I want harness-internal wakeups or Monitor activity to create a proper Run before output is persisted, so that autonomous Agent work has a complete lifecycle.
17. As a Task creator, I want long-running tool activity to keep the Task visibly alive, so that an active Turn is not mistaken for an idle Sandbox.
18. As a Task creator, I want Agent output to appear in native order, so that text, tool calls, and tool results form a correct conversation.
19. As a Task creator, I want each assistant response to appear as one assembled Message even when the CLI emits separate content blocks, so that the Chat does not show fragmented responses.
20. As a Task creator, I want token usage counted once per native Message, so that usage and cost remain accurate when a CLI repeats usage on every block.
21. As a Task creator, I want tool results folded into the assistant Message that issued the tool call, so that live and reloaded Chat views agree.
22. As a Task creator, I want subagent activity to retain its parent tool identity, so that subagent summaries and lazy transcripts remain attached correctly.
23. As a Task creator, I want new or unknown native block types preserved diagnostically, so that an upstream CLI update does not silently discard output.
24. As a maintainer, I want the coordinator to contain no Claude-versus-Codex branch, so that adding a harness does not modify Task lifecycle policy.
25. As a maintainer, I want one harness interface for production and tests, so that the interface is the test surface.
26. As a maintainer, I want harness-specific process behavior localized under that harness, so that changes to Codex do not require understanding Claude internals.
27. As a maintainer, I want Session continuity rules localized in the harness module, so that resume bugs can be fixed once.
28. As a maintainer, I want retry decisions outside the harness, so that error classification does not silently become product policy.
29. As a maintainer, I want backend publishing outside the harness, so that native adapters cannot bypass Run identity or event ordering.
30. As a maintainer, I want canonical source-event types rather than ad hoc provider JSON, so that every harness targets one documented contract.
31. As a maintainer, I want Message assembly exclusively in the backend, so that each harness does not reimplement grouping and usage deduplication.
32. As a maintainer, I want one terminal signal for every started execution, so that all success, failure, cancellation, and unexpected-exit paths close the Run.
33. As a maintainer, I want explicit backpressure at the harness seam, so that input or output is never silently dropped under load.
34. As a maintainer, I want at most one active Agent process enforced by the harness, so that process replacement cannot overlap and corrupt Session state.
35. As a maintainer, I want a scripted mock CLI adapter, so that coordinator and harness lifecycle behavior can be tested without paid or installed CLIs.
36. As a maintainer, I want production CLI adapters tested against the same harness contract, so that provider-specific behavior cannot violate shared ordering invariants.
37. As a maintainer, I want pure native-normalization tests driven by captured fixtures, so that upstream protocol changes are easy to diagnose.
38. As a maintainer, I want a new Agent harness to require a new private implementation and one composition entry, so that extension has high locality.
39. As a maintainer, I want no public trait introduced solely to mirror the two current implementations, so that the architecture avoids shallow indirection.
40. As a maintainer, I want private CLI ports only where production and mock adapters both exist, so that every seam is justified.
41. As a maintainer, I want focused files organized by process lifecycle, Session rules, normalization, identity, and policy, so that no replacement actor becomes another thousand-line file.
42. As a maintainer, I want file length treated as a design-review trigger rather than a rigid limit, so that we avoid both monoliths and meaningless pass-through files.
43. As a maintainer, I want the abandoned executor architecture deleted, so that there is one authoritative execution path for humans and coding agents to navigate.
44. As an operator, I want structured diagnostics to identify the harness, failure stage, error class, Session impact, and process exit, so that incidents can be triaged quickly.
45. As an operator, I want Sandbox restart recovery to restore Session state before pending input is delivered, so that resume behavior is deterministic.
46. As an operator, I want unexpected process EOF to become a classified terminal outcome, so that a Run never remains active silently.
47. As an operator, I want the harness to terminate an entire Agent process tree on cancellation, so that child shell commands cannot outlive the Turn.
48. As an operator, I want existing Claude and Codex behavior preserved during migration, so that the architectural change can roll out without user-facing regression.
49. As an API consumer, I want no new public HTTP contract for harness internals, so that this refactor does not expose Sandbox implementation details.
50. As an API consumer, I want existing Message reads and live upserts to converge on the same assembled result, so that clients need no new merge logic.

## Implementation Decisions

### Domain and lifecycle

- **Agent harness** is the canonical term for the execution implementation that controls an Agent process. “Agent type” and “Agent runtime” are retired terms in this design.
- A Task selects one Agent harness at creation. The selection is immutable for the Task lifetime. Running another harness requires another Task.
- A Sandbox creates one instance of the selected harness. The instance is in-memory and is recreated when the Sandbox process is recreated.
- An **Agent** is the CLI process controlled by the harness. At most one Agent process may be active in a Sandbox at a time.
- A **Session** is the resumable CLI conversation. It may span Turns, Agent process replacement, and Sandbox recreation.
- A **Run** remains the Sandbox-internal identity of one product Turn. The coordinator, not the harness, mints and owns it.
- Harness selection is resolved once when the Sandbox starts. Pending Messages do not change it. Compatibility with the separate Agent-config work must preserve this invariant even while legacy field names remain during migration.

### External seam

- The Agent harness is one concrete deep module. Callers do not receive Claude or Codex handles and do not match on the selected harness.
- The operational interface has three actions: submit input, cancel the active execution, and receive the next signal. Construction fixes the harness implementation for the Sandbox.
- The external interface is a bounded command/signal protocol. Bounded channels or an equivalent mechanism provide explicit backpressure.
- No public universal harness trait is required. Current implementation selection stays behind a private driver enum or equivalent closed composition mechanism.
- Harness-specific CLI ports are private internal seams. Each port is justified by a production adapter and a scripted mock adapter.

The following state-machine ordering came from the design prototype and is
normative because prose is less precise:

```text
Input opening a new execution:
  submit(input)
  → ExecutionStarted(session, cause=input)
  → InputAccepted(input_id)
  → Source*
  → ExecutionFinished(outcome)

Input joining an active compatible execution:
  submit(input)
  → InputAccepted(input_id)
  → Source*
  (no new ExecutionStarted)

Harness-internal execution:
  ExecutionStarted(session, cause=harness-internal)
  → Source*
  → ExecutionFinished(outcome)
```

- `ExecutionStarted` is emitted only after an Agent process is ready and the Session identity is known. The coordinator mints the Run when it receives this signal.
- `InputAccepted` means the Agent process accepted the input. The coordinator acknowledges pending Messages only after this signal and associates them with the current Run.
- Claude input injected into an active compatible process emits `InputAccepted` without another `ExecutionStarted`, so it joins the current Run.
- A harness that cannot inject while active queues input internally in order. After the current terminal signal, it starts or resumes a replacement Agent process, emits `ExecutionStarted`, and then accepts the queued input.
- A harness-internal wakeup emits `ExecutionStarted` before its first source event. It has no `InputAccepted` signal because no pending Message caused it.
- Every started execution emits exactly one terminal `ExecutionFinished`, including success, failure, cancellation, unexpected EOF, and unrecoverable native-protocol error.
- Input that fails before an execution starts produces `InputRejected` with its correlation identity and classified failure.
- Cancellation is idempotent. Cancellation while idle is a no-op; cancellation while active terminates the Agent process tree and produces one cancelled terminal outcome.

### Harness ownership

- The harness owns Agent process creation, compatibility checks, replacement, and termination.
- The harness owns Session start, explicit fresh start, resume, restored-Session handling, and classification of Session loss.
- The harness owns the difference between persistent input injection and one-shot per-Turn execution.
- The harness owns native protocol parsing and translation into typed canonical source events.
- The harness owns stable source-event identity. When a CLI reuses native identifiers after process replacement, the harness adds a stable collision-avoidance element without exposing provider details.
- The harness owns error facts: failure stage, stable class, diagnostic text, and whether Session state is intact, lost, or unknown.
- The harness does not own pending-Message fetch/batching, Run identity, Turn status, Message acknowledgement, retry count, backoff, circuit breaking, credential policy, backend publishing, or Message assembly.

### Coordinator ownership

- The coordinator is harness-neutral and consumes only harness commands and signals.
- The coordinator fetches and prepares pending Messages, including uploaded-file materialization and `start_fresh` intent.
- The coordinator mints a Run and marks the Turn active after `ExecutionStarted`.
- The coordinator acknowledges input after `InputAccepted`, using the known Session and current Run.
- The coordinator publishes normalized source events and liveness activity with the current Run identity.
- The coordinator closes the Run and Turn after the single terminal signal.
- Retry count, exponential backoff, circuit breaker behavior, and whether to request a fresh Session are coordinator policies driven by harness error classification.
- Credential recovery policy remains outside the harness. The harness exposes authentication failure facts and the coordinator decides whether refreshed credentials justify another attempt.

### Canonical source events and Message assembly

- Every source event represents exactly one native content block. The type shape must not permit a harness to return a vector of blocks as one assembled Message.
- Source events carry stable source-event identity, native Message grouping identity, Session identity, optional parent tool identity, role, one canonical block, usage as reported on that block, model where available, and bounded native diagnostics.
- Harnesses preserve native ordering and repeated usage. They do not coalesce content, fold tool results, or deduplicate usage globally.
- The backend is the only Message assembler. It groups assistant blocks by native Message identity, preserves block order, admits repeated usage once, folds tool results into the affected parent, and produces the derived Message upsert.
- Paginated reads and live derived upserts use the same assembler seam.
- Subagent rows remain excluded from top-level Message assembly and continue to use summary/progress plus on-demand transcript behavior under ADR-0003.
- The Sandbox-side assistant buffer, its per-turn usage set, and its pending flush behavior are deleted after backend assembly supports the canonical per-block contract.
- Source-event skeleton persistence and relay behavior remain governed by ADR-0003; full content remains a transient ingestion payload written once to Message storage.

### Errors, retries, and recovery

- Harness errors use stable classifications rather than provider error types at the external seam. Required classes cover authentication, rate limiting, unavailable Session, invalid configuration, invalid input, native-protocol failure, Agent process exit, unresponsive Agent process, and internal invariant failure.
- A harness error states Session impact separately from error class.
- The harness never exposes a `retryable` policy boolean. The coordinator maps stable facts to retry policy.
- Resume failure never silently creates a fresh Session. The coordinator explicitly chooses whether to request a fresh Session and the conversation receives the existing context-loss signal.
- Early native output that arrives before Session identity is known is buffered inside the harness and released in native order after `ExecutionStarted`.
- Command and signal queues must not silently drop data. Closed queues, backpressure, and invariant violations become explicit errors.

### Internal module shape

- The coordinator is split by owned responsibility: orchestration loop, pending-input delivery and acknowledgement, retry policy, and publishing.
- The Agent harness is split into its public protocol, shared ordering state machine, Session rules, and stable error/source-event types.
- Each production harness has a private subtree organized around launch configuration, process lifecycle, native normalization, native identity, and genuinely harness-specific behavior such as persistent internal Turns.
- Native normalizers are pure where practical. Process lifecycle and Session state are not mixed with payload mapping.
- Public module roots re-export the small interface and do not accumulate implementation logic.
- Roughly 200–300 production lines is a review signal. A file above that range is acceptable only when it still has one cohesive responsibility; it is not split into pass-through wrappers merely to meet a number.
- No new trait is introduced with only one production implementation. Test-only seams are private and must have a real test adapter in use.
- The retired, uncompiled executor and its duplicate Claude implementation are removed. Useful behavioral tests are moved to the new harness interface rather than preserving the obsolete architecture.

### Migration and rollout

- Build the new typed protocol and scripted mock first, then prove the shared ordering contract through the harness interface.
- Adapt Claude behind the seam while preserving subprocess, persistent transport, internal wakeup, prompt, cancellation, and Session behavior.
- Adapt Codex behind the same seam while preserving one-shot execution, thread resume, collision-safe identity, cancellation, and credential-recovery behavior.
- Move generic Run/Turn, acknowledgement, retry, and publishing behavior into the coordinator as harness branches disappear.
- Correct backend per-block Message assembly before removing Sandbox coalescing. During transition, use one explicit compatibility path and remove it once both sides use the canonical contract.
- Remove per-Message harness switching. Resolve the Task’s fixed harness when constructing the Sandbox harness instance.
- Delete the old actor branches and abandoned executor only after both production harnesses and the mock pass the same contract suite.
- Preserve existing public HTTP behavior and Message shapes throughout the migration.

## Testing Decisions

### Testing philosophy

- Good tests assert observable behavior through the highest stable interface. They do not inspect private driver enums, channel internals, process handles, queue fields, or helper call order.
- The Agent harness command/signal interface is the primary new test seam. It is the interface used by the coordinator and therefore the interface used by lifecycle tests.
- Tests replace shallow helper tests once equivalent behavior is covered through the harness interface. They do not layer duplicate tests onto both old and new architectures.
- Deterministic scripted CLI adapters are preferred for lifecycle coverage. Real CLI E2E tests are retained only for integration facts a mock cannot prove.

### Harness contract suite

- Run one shared contract suite against the scripted mock and each production harness adapter where deterministic execution is possible.
- Cover new execution ordering: started, accepted, zero-or-more source blocks, exactly one terminal outcome.
- Cover input injection into an active compatible process without opening a new execution.
- Cover ordered queueing for a one-shot harness and opening the next execution only after the prior terminal signal.
- Cover harness-internal execution opening before its first source block.
- Cover Session start, resume, explicit fresh start, unavailable Session, and process replacement with preserved Session.
- Cover input rejected before start and failure after start as distinct observable outcomes.
- Cover cancellation while idle, during startup, during output, and after terminal completion.
- Cover unexpected EOF, malformed native output, unresponsive process, and process-tree termination.
- Cover backpressure and closed-channel behavior without silent loss.
- Cover at-most-one-process and exactly-one-terminal invariants.
- Cover early output buffering until Session identity is known.

### Coordinator tests

- Use the scripted harness interface and an in-memory backend adapter to test the coordinator without a real CLI.
- Assert that Runs are minted only after `ExecutionStarted`.
- Assert that pending Messages are acknowledged only after `InputAccepted` and receive the correct Session and Run identities.
- Assert that injected input joins the active Run and queued input is assigned to the next Run.
- Assert that autonomous execution creates a Run without acknowledging a pending Message.
- Assert that retry, backoff, circuit breaker, credential recovery, context-loss fallback, and terminal failure follow coordinator policy for each stable error class.
- Assert that every terminal harness outcome closes the corresponding Run and Turn once.
- Assert that cancellation and duplicate notifications remain idempotent.
- Prior art: the existing Agent transport E2E tests, execution-flow integration tests, stop-recovery tests, and actor heartbeat tests.

### Native normalization tests

- Use captured Claude and Codex fixtures to test pure translation into canonical one-block source events.
- Assert stable native Message grouping identity, parent tool identity, role, model, usage, metadata, and collision avoidance.
- Assert that a native envelope containing multiple blocks produces multiple ordered source events.
- Assert that repeated usage is preserved for backend admission rather than removed in the harness.
- Assert that unknown native item types become bounded diagnostic blocks or diagnostics instead of disappearing.
- Prior art: existing Codex payload mapping tests and Claude SDK message tests.

### Backend Message assembly tests

- Test the backend through its persisted source-event ingestion and assembled Message read/upsert seam, preferably with real Postgres in the integration suite.
- Persist multiple source events with one native Message identity and assert one assembled UI Message with blocks in native order.
- Persist repeated usage on those events and assert usage is admitted once.
- Persist interleaved assistant and tool-result events and assert affected-parent resolution produces the same result for read and live upsert.
- Assert subagent rows remain lazy and do not enter the top-level Message payload.
- Assert duplicate source-event delivery is idempotent.
- Prior art: existing Message transform tests, affected-parent tests, source-event skeleton tests, and database-backed Message integration tests introduced for ADR-0002/0003.

### Sandbox recovery tests

- Use temporary directories as the local substitute for Session-state storage and an in-memory backend adapter for archive upload/download.
- Assert that a replacement Sandbox restores Session state before the harness accepts pending input.
- Assert that the selected harness remains fixed across replacement.
- Assert that missing or corrupt Session state produces classified context loss rather than an implicit harness change.
- Prior art: transcript-sync round-trip tests, interrupted-run recovery tests, and Sandbox lifecycle integration tests.

### Real CLI E2E tests

- Retain a small Claude subprocess smoke test, persistent Claude transport test, and Codex resume test to verify actual CLI launch/protocol compatibility.
- Do not use real CLI E2E tests to prove generic coordinator retry, acknowledgement, cancellation, or ordering behavior.
- A future harness must add at least one real launch/protocol smoke test plus pass the shared scripted contract suite.

### Mandatory verification

- Run the Sandbox crate formatter, clippy with warnings denied, and full crate tests.
- Run the backend formatter, clippy with warnings denied, and full crate tests for backend changes.
- Run the database integration tier for Message assembly and the Sandbox tier for cross-process lifecycle behavior.
- Run opt-in real Agent CLI E2E tests when credentials and executables are available; document any environment-gated tests not run locally.

## Out of Scope

- Implementing a third Agent harness. This spec makes that addition local but migrates only Claude Code and Codex.
- Dynamic loading of harness implementations or a public plugin interface for Agent harnesses.
- Agent-config CRUD, scope policy, administration UI, model catalog authoring, tool policy, or plugin composition; those belong to the separate Agent-config specification.
- Switching Agent harnesses within an existing Task.
- Generalizing all Agent bootstrap, credentials, workspace compatibility, or Session archive roots into profiles beyond what is necessary for this seam migration.
- Changing Sandbox runtimes such as Kubernetes or Docker.
- Changing external-system Adapters such as Slack, GitHub, Linear, Telegram, or Webhook.
- Replacing gRPC between the Sandbox and backend.
- Changing the public HTTP Message contract or moving Message assembly to clients.
- Reworking source-event retention, event-store pruning, or forensic logging beyond the accepted skeleton behavior.
- Changing Agent model semantics, Agent mode semantics, tool permissions, scheduled-job product behavior, or subagent transcript product behavior.
- Enforcing a rigid maximum line count. File length remains a review signal tied to responsibility and locality.

## Further Notes

- The term **Adapter** already names external-system integrations in the domain glossary. Within implementation discussion, CLI production/mock adapters describe their architectural role at a private seam; they are not product Adapter entities.
- ADR-0002 and ADR-0003 already describe backend-owned assembly. The current Sandbox assistant buffer is implementation drift to remove, not a reason to move the accepted seam.
- ADR-0004 records the fixed harness per Task decision.
- ADR-0005 records the harness/coordinator ownership split and per-block normalization rule.
- The strongest design-it-twice result was the minimal concrete command/signal module. A larger execution-ID protocol was rejected because it duplicates Run correlation and risks a shallow outcome interface. A borrow-scoped active-Turn interface was rejected because mid-Turn input, external cancellation, and harness-internal Turns would leak around it.
- Success means a maintainer can add a future harness implementation and its composition entry without changing coordinator lifecycle logic, backend Message assembly, or existing harness implementations.
- Success also means the central actor no longer contains harness-specific state or matches, both current production harnesses pass one shared interface contract, and no replacement production file becomes another mixed-responsibility thousand-line implementation.
