---
name: rust-dev-guidelines
description: Rust development and responsibility-review conventions for Moatless backend, sandbox, and shared crates. Use before running cargo fmt, rustfmt, cargo clippy, cargo build, cargo test, cargo check, cargo add or cargo nextest in this workspace, and when planning, writing, reviewing, restructuring, or changing Rust under crates/ or tooling/.
---

# Rust Development Guidelines

Apply the repository-specific rules below in addition to the workspace lints and
the verification commands in `AGENTS.md`. Do not turn current code patterns into
new conventions without checking that they remain sound.

## Start from the Boundary

1. Identify the owning crate and the runtime boundary affected by the change.
2. Load every applicable reference:
   - all Rust work → [file responsibilities](references/file-roles.md)
   - `crates/server/` → [backend.md](references/backend.md)
   - `crates/sandbox/` → [sandbox.md](references/sandbox.md)
   - `crates/` → [shared-crates.md](references/shared-crates.md)
   - running, adding, varying, or removing a cargo invocation, or configuring
     an editor against this workspace →
     [build-costs.md](references/build-costs.md)
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

### Fail loudly from a write-path trait default

A default method that returns `Ok(())` turns a missed override into a silent
no-op: events, acks or writes are dropped and nothing compiles differently. A
default on a write path returns an error such as
`Status::unimplemented("<method> not supported")`; a fake that needs success
overrides it in one line. A read-path default may answer as an empty store.

### Keep visibility narrow

Use the smallest visibility required by real consumers. Do not expose an item
publicly in anticipation of a future caller.

### Write comments to the repo rules

The comment rules in the root `AGENTS.md` apply. Rust specifics:

- Doc comments are Markdown: `-` lists, fenced blocks, tables, and a bold label
  such as `**Failure policy:**` are the house style. No HTML tags.
- The first line is the one-sentence summary. `clippy::too_long_first_doc_paragraph`
  caps it at 200 characters on a public item only. Failure semantics follow as prose:
  "Returns `None` when ...", "Panics when ...". Do not add `# Arguments`,
  `# Returns`, or `# Errors` sections. A `#` heading names a concern the
  reader must find, such as `# Concurrency` or `# Call this last`.
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
- After shortening a comment, re-read it against the code. Keep any clause
  that defends code which looks redundant (a second `ORDER BY`, an extra
  check), keep the qualifiers that scope a claim ("every other event a Task
  produces", not "every event"), and confirm the sentence still parses.

### Format with `cargo fmt`

```bash
cargo fmt --all
```

The tree is formatted, and `Rust clippy` fails a pull request that is not, so
run this rather than picking files. `rustfmt.toml` pins `edition` and
`style_edition`, so `cargo fmt`, a bare `rustfmt <file>` and an editor's
format-on-save all produce the same bytes.

`crates/codex-sdk` is its own workspace and `--all` does not reach it. Its path
dependencies live outside the repository, so `cargo` cannot read its manifest in
a sandbox and neither can CI; format it with
`find crates/codex-sdk -name '*.rs' | xargs rustfmt`, which picks up that
crate's own `rustfmt.toml`.

Naming a `mod.rs` (or any file declaring `mod`) formats its whole subtree, not
just that file, and `--skip-children` is nightly-only. So `rustfmt <file>` does
not keep a diff narrow.

## Completion Check

Before declaring the Rust change complete:

- confirm every changed Rust path was checked against its applicable reference;
- confirm file contents match their assigned responsibility, including callers
  outside the owning directory;
- confirm critical task, queue, error, and conversion behavior is explicit where
  the change touches those boundaries;
- run the crate-specific Clippy and test commands required by `AGENTS.md`;
- report any required check that could not be run.
