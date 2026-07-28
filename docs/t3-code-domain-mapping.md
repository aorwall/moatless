# T3 Code ↔ Moatless: domain mapping and terminology

This document defines how T3 Code's domain model maps onto Moatless's, which
vocabulary wins where, and what the phase-one UI RPC surface
(`backend/src/ui_rpc/mod.rs`) has to fill in next.

It complements [the local development runbook](./runbooks/t3-code-local-development.md),
which covers how to *run* T3 Code against this backend. This document covers what
the data flowing through it *means*. Domain terms are defined in
[`CONTEXT.md`](../CONTEXT.md).

**The deeper analysis lives in the T3 Code repository**, on its `moatless-adapter`
branch — `docs/reference/moatless-concept-map.md` walks every contract surface
feature-by-feature, and `.plans/moatless-convergence.md` proposes changing both
sides. This document is the Moatless-side summary and the terminology decision; it
does not repeat their findings. Where the two disagree, that is called out inline.
See [the orientation note](./t3-code-integration-start-here.md) for reading order.

Read against `soaplabs/t3code` @ `d67a67a` and this repository @ `ef4d860`.

## The two models are not variants of each other

Both systems model a container of run-configuration holding many agent
conversations, each made of turns driven by a provider session. They disagree
about where truth lives, and every downstream difference follows from that:

- **T3 Code derives state from events.** `orchestration_events` is append-only
  and authoritative; all nine `projection_*` tables are rebuildable folds keyed
  on a global `sequence`.
- **Moatless derives events from state.** `tasks`, `task_turns`, `task_messages`
  are authoritative. `migrations/0071_task_turns.sql` demoted the log
  deliberately, so that *"events become projections rather than the source of
  truth."*

The adapter is therefore an anti-corruption layer, not a translation table. It
is expected to serve T3 Code's contract from Moatless's rows without either side
adopting the other's storage model.

## Direction: Moatless vocabulary wins

Where a concept genuinely exists on both sides, T3 Code renames toward Moatless.
Three reasons, all structural:

1. **`thread` is already taken in this repository**, and it means *external
   channel thread*: `channel_threads` (`migrations/0002_channels.sql:30`),
   `external_thread_id`, `routing_mode = 'thread_based'`, and the subscription
   kinds `slack_thread` / `telegram_thread` / `connected_thread`
   (`migrations/0035_slack_telegram_subscription_cutover.sql`). Importing T3
   Code's Thread-as-conversation would overload a word that is live in the
   adapter subsystem. The reverse has no collision — T3 Code has no `Task`.
2. **Blast radius is asymmetric.** Moatless's `Task` spans 108 REST path
   templates, the Orval-generated TypeScript client, ~100 migrations, and adapter
   payloads. T3 Code's `Thread` is ~16.7k occurrences across ~1.1k files, but
   nearly all of them derive from `packages/contracts`, the single source of
   types for its server, web, and mobile targets. One rename there propagates by
   type error.
3. **This repository already has the written glossary.** `CONTEXT.md` plus a
   numbered ADR set. T3 Code has `docs/` but no glossary file.

Where a concept exists only in T3 Code, **T3 Code's name carries over unchanged**
— there is nothing to harmonize with.

## Mapping

| T3 Code | Moatless | Status |
| --- | --- | --- |
| `Thread` | `Task` | Genuine synonym. Rename in T3 Code. |
| `Turn` (`projection_turns`) | `Turn` (`task_turns`) | Already identical. Align status enums: `pending·running·interrupted·completed·error` vs `active·completed·error·stopped·interrupted`. |
| `Session` (`projection_thread_sessions`) | `Session` | Same concept — the resumable provider conversation. Moatless additionally splits out `Sandbox` as the durable pod-lifecycle intent; T3 Code's session row carries both concerns. |
| Message | `TaskMessage` + content blocks | T3 Code stores flat rows; Moatless stores one row with N blocks and folds tool results into the calling assistant message at assembly (ADR-0002). |
| Activity (`projection_thread_activities`) | `task_events` | **Not equivalent.** T3 Code's activities are a projection; `task_events` is history. |
| `Project` | `Workspace` | Mapped (`projectId ↔ workspaces.id`), but **not** a synonym — see below. Do not rename either. |
| `Environment` | — | T3 Code only. Keep the name. |
| Checkpoint, proposed plan, `settled`/`snoozed` | — | T3 Code only. Keep the names. |
| — | `Binding`, `Adapter`, `Loop`, `Schedule`, `Subscription` | Moatless only. |
| — | `Scope`, `Visibility` | Moatless only. `Scope` is two values (`Global`, `User`); ownership is `COALESCE(owner_user_id, created_by)`. `team_id` is **inert for access control** since `migrations/0097_scope_two_values.sql` — do not filter on it. |

### Do not rename `Project` to `Workspace`

A T3 Code Project **is a filesystem path** — one `workspace_root`, unique among
live projects, plus a `repositoryIdentity` derived read-only from the git remote.
A Moatless Workspace is run-configuration (image, setup commands, env vars,
servers, resources) composing 1..n repositories through `workspace_repos` with
placements and a designated primary.

The honest mapping is:

```
T3 Code Project  ≅  Moatless Workspace with exactly one WorkspaceRepo
```

Renaming Project to Workspace before T3 Code can compose repositories would put
the name on a capability that does not exist. Either deepen Project into real
repository composition first, or keep the name and document the projection.

### Decision: a T3 Project maps to a Workspace

**Settled.** `projectId ↔ workspaces.id`. This supersedes the Project→Repository
row in T3 Code's `docs/reference/moatless-concept-map.md` (`projectId ↔
task.repositoryId`), which is now stale.

Rationale: a Task already references `workspace_id`, so the Workspace mapping is
the direct one — a Repository-shaped Project would have to resolve to a Workspace
on every task creation anyway. Multi-repo Workspaces stay visible instead of being
flattened, and no Repository becomes unreachable, because **Default workspace**
already guarantees every Repository has one.

Field by field:

| `OrchestrationProject` | Moatless source |
| --- | --- |
| `id` | `workspaces.id` |
| `title` | `workspaces.name` |
| `workspaceRoot` | **Synthetic — see the hazard below.** |
| `repositoryIdentity` | The primary `WorkspaceRepo`'s Repository. `canonicalKey` is its normalized remote URL. |
| `defaultModelSelection` | `null`. Model selection is per Task/Turn in Moatless, not per Workspace. |
| `scripts[]` | `workspaces.servers` (`ServerConfig`). `name`/`label` → `name`, `start_commands` → `command`, `default` → `autoOpenPreview`. `previewUrl` has **no direct source**: `ServerConfig.port` is a `u16` (`backend/src/sandbox/models.rs:130`) and `ProjectScript.previewUrl` is an optional non-empty URL string (`orchestration.ts:192`). The adapter must synthesize the sandbox's published preview URL for that port, or omit the field — a bare port number is not a valid value. `setup_commands` are provisioning, **not** scripts — do not map them here. |
| `createdAt` / `updatedAt` | `workspaces.created_at` / `updated_at` |
| `deletedAt` | `null`. `workspaces.deleted` is a boolean and deleted rows are filtered from the snapshot, so no timestamp is needed. |

`workspaces.kind` (`blank` \| `single` \| `multi`) tells the adapter up front when a
Workspace cannot be represented faithfully as a single root.

### Hazard: `workspaceRoot` is an identity, not a label

T3 Code enforces **uniqueness of `workspaceRoot` across active projects** —
`commandInvariants.ts:94`: *"Active project '{id}' already exists for workspace root
'{root}'."* The client also derives its physical project key from it
(`${environmentId}:${normalizedPath}`), and uses that key for sidebar grouping,
ordering, and per-project overrides.

Moatless has no per-Workspace path. Repositories are cloned into
`{WORKSPACE_DIR}/{mount_name}` inside a per-Task sandbox, so `WORKSPACE_DIR` is
**the same string for every Workspace**. Reporting it verbatim would make every
project share one physical key and collapse into a single sidebar group, and would
be rejected outright on the write path.

The adapter must therefore synthesize a **stable, unique, per-Workspace** root
(encoding `workspaces.id`). It is nominal either way — the real path only exists
once a Task has a sandbox.

Related consequence: T3's project-scoped file operations
(`projects.listEntries`/`readFile`/`writeFile`) have no target until a Task exists,
because files live in a per-Task sandbox. This is the same gap convergence §2.2
describes as *"every method that is implicitly the environment's today gains a
`threadId`"*.

### Do not rename the persisted event literals

`thread.create`, `thread.settle`, `thread.snooze` and the rest are persisted
string literals in T3 Code's `orchestration_events.event_type` — an append-only,
never-pruned table that is its only source of truth. Renaming them breaks replay
of all history.

When `Thread → Task` lands, move the **type layer** in `packages/contracts` and
leave the wire and storage strings alone, or ship an alias map applied at read
time. Same discipline as a database column rename.

## Two axes of federation

A recurring source of confusion: both systems are "multi-something," but not the
same something.

- **T3 Code federates over machines you own.** `environmentId` is assigned
  *client-side*, one per connection target (`Primary`, `Bearer`, `Relay`, `Ssh`).
  No server table mentions it. Ids are only unique within an environment, so all
  cross-environment references are scoped `{environmentId, projectId}`. Projects
  are grouped across machines client-side by normalized git remote URL.
- **Moatless federates over access scope.** Rows carry `scope` (`Global` or
  `User`) plus ownership, and every query is filtered by a `ScopeContext`. Teams
  are an org concept and grant no resource access (ADR 0006).

T3 Code has nowhere to express *who may see this*; Moatless has nowhere to
express *this task lives on a machine I am not connected to right now*. The
adapter presents Moatless as **exactly one T3 environment**, and access control
stays entirely on the Moatless side.

## What phase two has to fill

Phase one returns a well-formed but empty snapshot
(`ui_rpc/mod.rs:553`) and an empty replay (`orchestration.replayEvents → []`),
and already advertises `shellResumeCompletionMarker` /
`threadResumeCompletionMarker` in `server_config`. The wire shape is fixed by T3
Code's contract:

| Snapshot field | Moatless source |
| --- | --- |
| `projects[]` | `workspaces` joined through `workspace_repos`, scope-filtered. One project per workspace; `workspaceRoot` is **synthetic and per-Workspace** — not the primary repo's mount path. See [the hazard](#hazard-workspaceroot-is-an-identity-not-a-label). |
| `threads[]` | `tasks`, scope-filtered, non-deleted, projected to `OrchestrationThreadShell`. |
| `snapshotSequence` | See below. |
| `updatedAt` | Wall clock at assembly. |

### The sequence is the load-bearing problem

T3 Code's client is a **sequence-fold client**: its shell reducer drops any event
whose `sequence <= snapshotSequence`, and resumption is `subscribeShell({afterSequence})`.
Moatless has no monotonic per-tenant sequence and explicitly no SSE replay — its
own frontend calls `invalidateQueries({refetchType: 'active'})` on reconnect.

The shape of the fix is favourable. T3 Code's shell stream does not fold event
payloads — it re-reads the projection row and ships a whole shell object, which
the client applies as upsert-by-id behind the sequence guard. So the adapter does
not need Moatless's events to be authoritative. It needs one question answered:

> which `task_id`s have changed since sequence `N`

and `task_events (id, task_id)` is the obvious place to answer it from. Both
systems already converged on *idempotent full upsert keyed on a stable id* — T3
Code's shell re-read, and Moatless's derived `message.upsert` keyed on the
persisted `uuid` (ADR-0002) — which is what makes this tractable at all.

**But `task_events.id` is not a safe cursor as-is, and this must be settled before
anything is built on it.** A `bigserial` is assigned at `INSERT` and made visible
at `COMMIT`, so it can be read out of order: two writers take ids 100 and 101, 101
commits first, a reader polling `id > 99` advances to 101 and never sees 100. A
naive `MAX(task_events.id)` at snapshot assembly has exactly this bug, and it fails
silently — the client believes it is caught up. T3 Code's own server names the same
hazard from the other side (`ws.ts:1226`: attach the live buffer before querying the
snapshot, or an event published mid-query is lost).

Three options, analysed in T3 Code's `.plans/moatless-convergence.md` §3.5:

- serve catch-up from the **live consumer's buffer** rather than a fresh `SELECT`,
  so the reader never straddles a commit boundary;
- hold the cursor below the oldest in-flight transaction (`pg_snapshot_xmin`),
  trading latency for correctness;
- allocate the sequence **inside the transaction** from a per-task counter, so
  ordering and visibility coincide.

The third also resolves a smaller mismatch: T3's `sequence` is a
`NonNegativeInt` (a JS number) while `task_events.id` is `i64`.

Worth noting independently: today's SSE lag is *silent* — `sse/routes.rs` logs
`SSE client lagged, skipping events` and drops frames with no way for the client
to detect it. Adding a sequence makes that detectable, which is a win regardless
of what else lands.

### Known gaps to resolve

- **Pagination.** T3 Code's shell snapshot has no pagination and holds every
  non-archived thread resident. Acceptable for one developer's machine; it will
  not survive a tenant. Windowing the shell is the one change required on the T3
  Code side.
- **Lifecycle mismatch.** T3 Code's `settled`/`snoozed` are client-derived and
  clock-dependent. Moatless's `TaskStatus` is server-durable `Open·Closed·Error`.
  Decide whether the adapter synthesizes settlement or T3 Code hides the control
  when the environment does not advertise the capability.
- **Turn status enums** differ by two values and need an explicit mapping.
- **`task_events` retention.** Unpruned today. If it is ever pruned, resume
  windows break silently; the adapter must fall back to a full re-snapshot past a
  configured gap, as T3 Code's own server does.

## Naming rule for the adapter

The wire contract is T3 Code's, so `ui_rpc` emits `projects`, `threads`, and
`snapshotSequence` regardless of internal naming. **Terminology harmonization
does not change the wire.** Inside the backend, Moatless names apply. The
translation belongs in `ui_rpc` and nowhere else — that is the whole point of an
anti-corruption layer.

## Suggested order

1. Extend `CONTEXT.md` with the T3 Code-only concepts and the explicit
   non-equivalences recorded here.
2. Build out `ui_rpc` against current names on both sides. This is where the
   mapping gets tested, and it is reversible.
3. Only then rename `Thread → Task` in T3 Code's `packages/contracts`, storage
   literals untouched.
4. Decide user-visible copy separately — "thread" vs "task" in the UI is a
   product call, not a schema one.

Renaming before step 2 would commit to a mapping that has not been exercised yet.
