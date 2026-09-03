---
name: rust-dev-guidelines
description: Rust development conventions for Moatless backend, sandbox, and shared crates. Use when planning, writing, reviewing, or changing Rust under backend/, sandbox/, or crates/.
---

# Rust Development Guidelines

Apply the repository-specific rules below in addition to the workspace lints and
the verification commands in `AGENTS.md`. Do not turn current code patterns into
new conventions without checking that they remain sound.

## Start from the Boundary

1. Identify the owning crate and the runtime boundary affected by the change.
2. Load every applicable reference:
   - `backend/` → [backend.md](references/backend.md)
   - `sandbox/` → [sandbox.md](references/sandbox.md)
   - `crates/` → [shared-crates.md](references/shared-crates.md)
   - adding, varying, or removing a cargo invocation, or configuring an editor
     against this workspace → [build-costs.md](references/build-costs.md)
3. Define a failing test or reproducible check before implementation.
4. For a new or materially changed module, interface, or dependency seam, use
   the `codebase-design` skill before settling the design.
5. Make the smallest change that satisfies the behavior, then run the owning
   crate's mandatory Clippy and test commands from `AGENTS.md`.

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

These are properties the code must have, not prose it must carry. Where a named
policy field, a type, or a test already states one, that is where it is stated;
what is left over goes in the nearest `AGENTS.md`, once. A module doc narrating
all five is the essay the root `AGENTS.md` comment rule forbids.

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

### Write comments to the repo rules

The comment rules in the root `AGENTS.md` apply. Rust specifics:

- Doc comments are Markdown: `-` lists, fenced blocks, tables, and a bold label
  such as `**Failure policy:**` are the house style. No HTML tags.
- The first line is the one-sentence summary. Failure semantics follow as
  prose: "Returns `None` when ...", "Panics when ...". Do not add
  `# Arguments`, `# Returns`, or `# Errors` sections. A `#` heading names a
  concern the reader must find, such as `# Concurrency` or `# Call this last`.
- Link to the owner of a fact with an intra-doc link: `` [`Type::member`] ``
  or `` [`crate::module::item`] ``. `cargo doc` reports a link that does not
  resolve.
- `clippy::doc_markdown` enforces backticks around identifiers. A product name
  that trips it goes in `doc-valid-idents` in `clippy.toml`, not in backticks.
- A `//!` module doc owns the module's invariant and names its sibling
  `AGENTS.md` for the rest. It does not restate it.
- A trait method's doc is the whole contract, with or without a default body.
- A fenced block in a doc comment is a doctest unless it is tagged `text` or
  `ignore`. Tag it unless it should compile and run.
- The `// reason:` comment on a `ScopeContext::system` call states the cause
  only.

### Format single files, never the crate

Do not run `cargo fmt -p <crate>`. No crate here has a `rustfmt.toml`, and
`backend`, `sandbox` and every crate under `crates/` except `codex-sdk` declare
`edition = "2021"` — so `cargo fmt` formats them to the 2021 style edition,
while newer modules inside them are written in the 2024 one. The two disagree on
import ordering, so a crate-wide run rewrites unrelated files underneath your
change. On `moatless-backend` today that is 249 diff hunks; one refactor
lost real time unpicking 72 rewritten files out of its diff.

`crates/codex-sdk` is already `edition = "2024"` and does not have this split.

Format the files you actually touched:

```bash
rustfmt --edition 2021 --style-edition 2024 <file>...
```

That is a no-op on already-2024 modules such as
`backend/src/adapters/providers/github_pr/`, which is the shape to match.

Naming a `mod.rs` (or any file declaring `mod`) formats its whole subtree, not
just that file — `rustfmt sandbox/src/init/mod.rs` rewrote all eleven of its
siblings. `--skip-children` is nightly-only, so on stable the check is
`git status` afterwards: revert every file you did not touch.

If a `--check` run reports diffs in files you did not touch, that is the split
above, not your change. Leave them.

## Completion Check

Before declaring the Rust change complete:

- confirm every changed Rust path was checked against its applicable reference;
- confirm critical task, queue, error, and conversion behavior is explicit where
  the change touches those boundaries;
- run the crate-specific Clippy and test commands required by `AGENTS.md`;
- report any required check that could not be run.
