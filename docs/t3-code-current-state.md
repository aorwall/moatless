# T3 Code integration current state

> **Updated:** 2026-07-29
> **Scope:** Moatless-side T3 compatibility surface in `crates/t3code`

This document replaces the Moatless-side T3 planning specs that used to live in
`specs/`. It records what is still relevant now: the shipped backend contract
surface, the sidebar projection, the durable listing feed, and the auth boundary.

## Current Solution

Moatless exposes an opt-in T3 compatibility surface that translates Moatless
Workspaces and Tasks into T3 Projects and Threads at the wire boundary only.
`crates/t3code/src/lib.rs:1` states the boundary: T3 vocabulary belongs in the
compatibility module and on the wire, not behind it.

The backend no longer returns an empty shell. The HTTP shell snapshot reads the
viewer-specific Workspace and Task rows, projects them to T3's shell shapes, and
returns the listing position the snapshot was read at
(`crates/t3code/src/lib.rs:282`, `crates/t3code/src/lib.rs:322`).

## Runtime Surface

The compatibility router is mounted only when `T3_UI_RPC_ENABLED` is true
(`crates/t3code/src/lib.rs:134`). When enabled, it exposes these routes
(`crates/t3code/src/lib.rs:220`):

| Route | Auth | Current behavior |
|---|---|---|
| `/.well-known/t3/environment` | Public | Returns the T3 environment descriptor. |
| `/oauth/token` | Public — the pairing code is the credential | Redeems a pairing code for a bearer token. |
| `/api/auth/session` | Moatless session or bearer token | Returns a T3 session bootstrap payload for the authenticated viewer. |
| `/api/auth/websocket-ticket` | Moatless session or bearer token | Issues a short-lived ticket for the socket. |
| `/api/orchestration/shell` | Moatless session or bearer token | Returns the current shell snapshot. |
| `/ws` | Ticket, or session plus an exact Origin allow-list | Serves T3 JSON RPC frames and shell subscriptions. |

`/oauth/token` is the one route that does not live under `/api`, `/ws`, or
`/.well-known`, because the T3 client builds it from the origin and nothing else
(`apps/mobile/src/features/cloud/linkEnvironment.ts:546`). A deployment that
routes to the backend by path prefix therefore needs an edge rule for it by
name; without one it lands on whatever serves the SPA, and a POST to a static
file server answers `405` — which reads like a rejected request rather than a
request that never arrived.

The chart carries that rule as the `-t3-surface` `IngressRoute`
(`k8s/helm/moatless-vibe/templates/app-ingressroute.yaml`), matching `/ws`,
`/.well-known/t3`, and `/oauth/token` at a priority above the SPA catch-all. A
new T3 route outside `/api` has to be added there as well as to the router.

The published `environmentId` is the configured deployment name plus a digest of
the database identity (`crates/t3code/src/lib.rs:173`). That makes a restored or
replaced database become a different T3 environment instead of reusing a cached
listing built against another database.

## Shell Snapshot

The shell snapshot is viewer-specific.

| T3 concept | Moatless source | Current rule |
|---|---|---|
| Project | Workspace | Every Workspace the viewer may read, plus any Workspace referenced by a listed Task. |
| Thread | Task | Every Task the viewer follows, may still read, and that is still open (`status = 'open'`; closed and errored Tasks are left out). |
| Sequence | Listing position | Read before the rows so a reflected change is never hidden behind a newer cursor. |

The asymmetric filtering is intentional. A sidebar is the work a person follows,
not every row they can read. Workspaces use the broader readable-plus-referenced
rule because T3 silently drops a Thread whose Project is missing
(`crates/t3code/src/lib.rs:307`).

The conversation query builds one row per Task with the derived fields needed by
the projection: newest Turn, Sandbox lifecycle, latest human-authored message,
model candidates, and pending user-input or plan tool calls
(`crates/t3code/src/dao.rs:22`, `crates/t3code/src/dao.rs:151`).

## Projection Rules

Projection to T3 happens in `crates/t3code/src/projection.rs`.

| Field group | Current behavior |
|---|---|
| Project root | Synthesizes `/moatless/workspaces/<workspace-id>` because T3 treats root path as identity and Moatless has no real per-Workspace checkout path (`crates/t3code/src/projection.rs:22`). |
| Thread title | Uses `tasks.name`, falling back to `task_id` when blank or absent (`crates/t3code/src/projection.rs:200`). |
| Runtime mode | Always reports `full-access`; Moatless has no narrower T3 runtime-mode equivalent (`crates/t3code/src/projection.rs:53`). |
| Worktree path | Reports `/opt/moatless/workspace` only when a Sandbox exists and has not been removed (`crates/t3code/src/projection.rs:370`). |
| Latest turn | Uses newest Moatless Turn; `turnId` is `<task_id>:<turn_number>` (`crates/t3code/src/projection.rs:237`). |
| Turn state | Maps active and Moatless-interrupted to T3 `running`, completed to `completed`, error to `error`, and stopped to T3 `interrupted` (`crates/t3code/src/projection.rs:272`). |
| Session | Folds Sandbox desired state and newest Turn openness into T3 session status (`crates/t3code/src/projection.rs:313`). |
| Model | Uses newest assistant-reported model, then requested model, then deployment default for the agent kind, then `unknown` with a warning (`crates/t3code/src/projection.rs:382`). |
| Unsupported T3 fields | Leaves settlement, snoozing, archived timestamp, interaction mode, checkpoints, messages, and full activity projection out of the shell row (`crates/t3code/src/projection.rs:189`). |

`latestUserMessageAt` deliberately counts only messages authored by a person.
Agent output does not reorder the sidebar continuously, and the query does not
use `tasks.last_message_at` because that field also moves for agent/system
activity (`crates/t3code/src/dao.rs:53`, `crates/t3code/src/dao.rs:183`).

## Listing Feed

T3's shell client is sequence-based. Moatless therefore has a dedicated listing
change record instead of trying to reuse `task_events`.

`task_listing` owns the feed position. A Task mutation records a listing-visible
change in the same transaction as the mutation, allocating from one
deployment-wide counter and calling the allocator last to avoid deadlocks.
Workspace state is snapshot-owned: Workspace mutations do not advance this
cursor, and clients observe them on their next snapshot or rebuild
(`crates/tasks/src/task_listing/mod.rs:1`, `crates/tasks/src/task_listing/dao.rs:8`).

The resume rule has three rebuild cases (`crates/tasks/src/task_listing/models.rs:66`):

| Case | Why rebuild |
|---|---|
| Database identity changed | The same numeric position could now name a different change. |
| Client cursor is ahead of the current head | No real change can carry that cursor. |
| Client cursor is outside the resume window | The retained records may no longer cover the gap. |

Live delivery is Task-only and is not a diff stream. `ShellFeed` reads recorded
changes, coalesces multiple changes for the same Task, re-reads the whole current
row through the same filters as the snapshot, and sends `thread-upserted`,
`thread-removed`, or a full `snapshot`
(`crates/t3code/src/feed.rs:1`, `crates/t3code/src/feed.rs:100`).

If a changed Thread enters a Project the client does not hold, the feed sends a
fresh snapshot because T3 cannot apply the two necessary rows at the same
position without dropping one (`crates/t3code/src/feed.rs:218`).

## WebSocket Protocol

The WebSocket route requires a valid Moatless identity and an Origin that one
entry of `T3_UI_RPC_ALLOWED_ORIGINS` admits (`crates/t3code/src/lib.rs:603`).
An entry matches exactly, unless its host is `*.<domain>` — then it admits any
host under that domain on the same scheme and default port, which is how a
deployment covers per-workspace sandbox preview hostnames that do not exist
until the workspace does.

The frame handler accepts T3 JSON RPC envelopes, validates request ids, supports
ping/ack/interrupt/eof, and starts long-lived subscription producers
(`crates/t3code/src/lib.rs:784`). The implemented request tags are mostly
bootstrap and subscription surfaces:

| Tag | Current behavior |
|---|---|
| `server.probe`, `server.getConfig`, `server.getSettings` | Return bootstrap/config payloads. |
| `orchestration.getArchivedShellSnapshot` | Returns readable projects and no archived threads. |
| `orchestration.replayEvents` | Returns an empty array; T3 clients resume shell state through `orchestration.subscribeShell`. |
| `orchestration.subscribeShell` | Sends catch-up or snapshot, then polls the listing feed every 250 ms. |
| `subscribeServerConfig`, `subscribeServerLifecycle`, `subscribeAuthAccess` | Return static snapshot/welcome payloads. |
| Terminal, preview, VCS, discovered-server subscriptions | Start empty streams. |

The shell subscription survives transient poll errors without advancing the
cursor, so the next poll covers the same batch (`crates/t3code/src/lib.rs:1034`).

## Auth State

The Moatless backend side has the correct auth boundary: discovery is public;
session bootstrap, shell bootstrap, and socket traffic require a validated
Moatless identity. Local development relies on a normal `moatless_session`
browser cookie shared across ports on `localhost`
(`docs/runbooks/t3-code-local-development.md:34`).

A client on another device pairs instead of borrowing that cookie. A logged-in
browser session mints a single-use code; the client redeems it at `/oauth/token`
for an ordinary Moatless API key, which is why a paired device appears in the
API Keys list and is unpaired by revoking it there
(`backend/src/auth/pairing.rs`). The socket takes a short-lived signed ticket in
the query string, because a React Native `WebSocket` sends no headers at all —
and a ticketed upgrade skips the Origin allow-list, which exists to defend the
cookie rather than a bearer token that had to be deliberately fetched.

## Known Gaps

| Gap | Current state |
|---|---|
| Full T3 commands | Most non-shell methods are unsupported or return empty bootstrap streams. |
| Terminal, preview, VCS, and file operations | T3 methods are environment-scoped, while Moatless worktrees are per Task/Sandbox; the contract still needs a conversation target. |
| Archived conversations | Shell rows omit `archivedAt`; the archived snapshot returns projects and no threads. |
| Settlement and snoozing | No Moatless equivalent is projected. |
| Per-row action permission | T3 has no shell field that distinguishes read access from control access. |
| Shell pagination | The snapshot is not paginated. |
| Browser auth UX in T3 | The Moatless backend enforces session auth; T3 still needs its own Moatless-style login flow for remote deployments. |
