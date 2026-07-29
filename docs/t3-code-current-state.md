# T3 Code integration current state

> **Updated:** 2026-07-29
> **Scope:** Moatless-side T3 compatibility surface in `backend/src/ui_rpc`

This document replaces the Moatless-side T3 planning specs that used to live in
`specs/`. It records what is still relevant now: the shipped backend contract
surface, the sidebar projection, the durable listing feed, and the auth boundary.

## Current Solution

Moatless exposes an opt-in T3 compatibility surface that translates Moatless
Workspaces and Tasks into T3 Projects and Threads at the wire boundary only.
`backend/src/ui_rpc/mod.rs:1` states the boundary: T3 vocabulary belongs in the
compatibility module and on the wire, not behind it.

The backend no longer returns an empty shell. The HTTP shell snapshot reads the
viewer-specific Workspace and Task rows, projects them to T3's shell shapes, and
returns the listing position the snapshot was read at
(`backend/src/ui_rpc/mod.rs:282`, `backend/src/ui_rpc/mod.rs:322`).

## Runtime Surface

The compatibility router is mounted only when `T3_UI_RPC_ENABLED` is true
(`backend/src/ui_rpc/mod.rs:134`). When enabled, it exposes four routes
(`backend/src/ui_rpc/mod.rs:220`):

| Route | Auth | Current behavior |
|---|---|---|
| `/.well-known/t3/environment` | Public | Returns the T3 environment descriptor. |
| `/api/auth/session` | Moatless session | Returns a T3 session bootstrap payload for the authenticated viewer. |
| `/api/orchestration/shell` | Moatless session | Returns the current shell snapshot. |
| `/ws` | Moatless session + exact Origin allow-list | Serves T3 JSON RPC frames and shell subscriptions. |

The published `environmentId` is the configured deployment name plus a digest of
the database identity (`backend/src/ui_rpc/mod.rs:173`). That makes a restored or
replaced database become a different T3 environment instead of reusing a cached
listing built against another database.

## Shell Snapshot

The shell snapshot is viewer-specific.

| T3 concept | Moatless source | Current rule |
|---|---|---|
| Project | Workspace | Every Workspace the viewer may read, plus any Workspace referenced by a listed Task. |
| Thread | Task | Every Task the viewer follows and may still read. |
| Sequence | Listing position | Read before the rows so a reflected change is never hidden behind a newer cursor. |

The asymmetric filtering is intentional. A sidebar is the work a person follows,
not every row they can read. Workspaces use the broader readable-plus-referenced
rule because T3 silently drops a Thread whose Project is missing
(`backend/src/ui_rpc/mod.rs:307`).

The conversation query builds one row per Task with the derived fields needed by
the projection: newest Turn, Sandbox lifecycle, latest human-authored message,
model candidates, and pending user-input or plan tool calls
(`backend/src/ui_rpc/dao.rs:22`, `backend/src/ui_rpc/dao.rs:151`).

## Projection Rules

Projection to T3 happens in `backend/src/ui_rpc/projection.rs`.

| Field group | Current behavior |
|---|---|
| Project root | Synthesizes `/moatless/workspaces/<workspace-id>` because T3 treats root path as identity and Moatless has no real per-Workspace checkout path (`backend/src/ui_rpc/projection.rs:22`). |
| Thread title | Uses `tasks.name`, falling back to `task_id` when blank or absent (`backend/src/ui_rpc/projection.rs:200`). |
| Runtime mode | Always reports `full-access`; Moatless has no narrower T3 runtime-mode equivalent (`backend/src/ui_rpc/projection.rs:53`). |
| Worktree path | Reports `/opt/moatless/workspace` only when a Sandbox exists and has not been removed (`backend/src/ui_rpc/projection.rs:370`). |
| Latest turn | Uses newest Moatless Turn; `turnId` is `<task_id>:<turn_number>` (`backend/src/ui_rpc/projection.rs:237`). |
| Turn state | Maps active and Moatless-interrupted to T3 `running`, completed to `completed`, error to `error`, and stopped to T3 `interrupted` (`backend/src/ui_rpc/projection.rs:272`). |
| Session | Folds Sandbox desired state and newest Turn openness into T3 session status (`backend/src/ui_rpc/projection.rs:313`). |
| Model | Uses newest assistant-reported model, then requested model, then deployment default for the agent kind, then `unknown` with a warning (`backend/src/ui_rpc/projection.rs:382`). |
| Unsupported T3 fields | Leaves settlement, snoozing, archived timestamp, interaction mode, checkpoints, messages, and full activity projection out of the shell row (`backend/src/ui_rpc/projection.rs:189`). |

`latestUserMessageAt` deliberately counts only messages authored by a person.
Agent output does not reorder the sidebar continuously, and the query does not
use `tasks.last_message_at` because that field also moves for agent/system
activity (`backend/src/ui_rpc/dao.rs:53`, `backend/src/ui_rpc/dao.rs:183`).

## Listing Feed

T3's shell client is sequence-based. Moatless therefore has a dedicated listing
change record instead of trying to reuse `task_events`.

`task_listing` owns the feed position. A Task or Workspace mutation records a
listing-visible change in the same transaction as the mutation, allocating from
one deployment-wide counter and calling the allocator last to avoid deadlocks
(`backend/src/task_listing/mod.rs:1`, `backend/src/task_listing/dao.rs:8`).

The resume rule has three rebuild cases (`backend/src/task_listing/models.rs:66`):

| Case | Why rebuild |
|---|---|
| Database identity changed | The same numeric position could now name a different change. |
| Client cursor is ahead of the current head | No real change can carry that cursor. |
| Client cursor is outside the resume window | The retained records may no longer cover the gap. |

Live delivery is not a diff stream. `ShellFeed` reads recorded changes, coalesces
multiple changes for the same subject, re-reads the whole current row through the
same filters as the snapshot, and sends `thread-upserted`, `thread-removed`,
`project-upserted`, `project-removed`, or a full `snapshot`
(`backend/src/ui_rpc/feed.rs:1`, `backend/src/ui_rpc/feed.rs:100`).

If a changed Thread enters a Project the client does not hold, the feed sends a
fresh snapshot because T3 cannot apply the two necessary rows at the same
position without dropping one (`backend/src/ui_rpc/feed.rs:218`).

## WebSocket Protocol

The WebSocket route requires a valid Moatless identity and an exact Origin match
against `T3_UI_RPC_ALLOWED_ORIGINS` (`backend/src/ui_rpc/mod.rs:603`).

The frame handler accepts T3 JSON RPC envelopes, validates request ids, supports
ping/ack/interrupt/eof, and starts long-lived subscription producers
(`backend/src/ui_rpc/mod.rs:784`). The implemented request tags are mostly
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
cursor, so the next poll covers the same batch (`backend/src/ui_rpc/mod.rs:1034`).

## Auth State

The Moatless backend side has the correct auth boundary: discovery is public;
session bootstrap, shell bootstrap, and socket traffic require a validated
Moatless identity. Local development relies on a normal `moatless_session`
browser cookie shared across ports on `localhost`
(`docs/runbooks/t3-code-local-development.md:34`).

The removed remote-auth spec is still relevant as T3-side product direction, but
not as shipped Moatless backend state. The backend expects T3 to use the
Moatless browser session; it does not implement T3 pairing and does not expect a
browser API key proxy to stand in for a person.

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
