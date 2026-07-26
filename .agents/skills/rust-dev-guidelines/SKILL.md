---
name: rust-dev-guidelines
description: Rust development conventions for Moatless backend, sandbox, and shared crates. Use when planning, writing, reviewing, or changing Rust under backend/, sandbox/, or crates/.
---

# Rust Development Guidelines

Apply the repository-specific rules below in addition to the workspace lints and
the verification commands in `CLAUDE.md`. Do not turn current code patterns into
new conventions without checking that they remain sound.

## Start from the Boundary

1. Identify the owning crate and the runtime boundary affected by the change.
2. Load every applicable reference:
   - `backend/` → [backend.md](references/backend.md)
   - `sandbox/` → [sandbox.md](references/sandbox.md)
   - `crates/` → [shared-crates.md](references/shared-crates.md)
3. Define a failing test or reproducible check before implementation.
4. For a new or materially changed module, interface, or dependency seam, use
   the `codebase-design` skill before settling the design.
5. Make the smallest change that satisfies the behavior, then run the owning
   crate's mandatory Clippy and test commands from `CLAUDE.md`.

## Core Rules

### Keep blocking work off async workers

Recursive filesystem traversal, blocking process or system calls, and
substantial CPU work invoked from async code must use async APIs or a bounded
`spawn_blocking` boundary. A standard mutex is appropriate when its guard never
crosses `.await`; use an async mutex only when awaiting while locked is
necessary.

### Own critical spawned work

A task that owns a server, consumer, lifecycle transition, durable
synchronization, request admission, or required cleanup must expose termination
to a supervisor, liveness/readiness, or its caller and participate in
cancellation and shutdown. Detached work is allowed only when loss is
intentional and ordinary errors are handled inside the task.

### Define admission and overload semantics

Every queue or asynchronous handoff must state:

- when the producer is acknowledged;
- what is durable;
- how retries and deduplication work;
- what happens at capacity;
- how shutdown drains.

Do not claim hard boundedness while allowing an unbounded ingress or an uncapped
priority class.

### Preserve error causes

Reusable and domain APIs expose meaningful error variants and retain underlying
errors as sources. Application entry points and glue may use boxed errors when
callers do not need to classify failures. Redact only at the transport boundary,
where the stable client response is produced and the causal error is logged
once.

### Use fallible conversions for partial mappings

Use `From` only for lossless, infallible conversion. Unknown or unrepresentable
wire values use `TryFrom` or an explicitly narrower transport type and are
handled at the boundary.

### Make lint suppression self-checking

Prefer `#[expect(lint, reason = "...")]` when a lint is expected at that exact
site. Use `#[allow(lint, reason = "...")]` only when conditional expansion or
configuration means the lint may not fire. Remove obsolete suppressions; never
add an unreasoned local suppression.

### Keep visibility narrow

Use the smallest visibility required by real consumers. Do not expose an item
publicly in anticipation of a future caller.

## Completion Check

Before declaring the Rust change complete:

- confirm every changed Rust path was checked against its applicable reference;
- confirm critical task, queue, error, and conversion behavior is explicit where
  the change touches those boundaries;
- run the crate-specific Clippy and test commands required by `CLAUDE.md`;
- report any required check that could not be run.
