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
   - reviewing a pull request → [review.md](references/review.md)
3. Define a failing test or reproducible check before implementation.
4. Make the smallest change that satisfies the behavior, then run the owning
   crate's mandatory Clippy and test commands from `AGENTS.md`.

## Core Rules

### Keep blocking work off async workers

Recursive filesystem traversal, blocking process or system calls, and
substantial CPU work invoked from async code must use async APIs or a bounded
`spawn_blocking` boundary. `clippy::await_holding_lock` rejects a standard guard
held across `.await`; use an async mutex only when awaiting while locked is
necessary.

### Own critical spawned work

A task that owns a server, consumer, lifecycle transition, durable
synchronization, request admission, or required cleanup must expose termination
to a supervisor, liveness/readiness, or its caller and participate in
cancellation and shutdown. Detached work is allowed only when loss is
intentional and ordinary errors are handled inside the task.

Give owned tasks a `tokio_util::task::TaskTracker` and a `CancellationToken`,
so shutdown can cancel, `close()` and `wait()` them. A `spawn_blocking` closure
cannot be aborted once it runs: pass it a stop flag or the token, check it
between units of work, and bound its input.

### Keep `select!` branches cancel-safe

`tokio::select!` and `tokio::time::timeout` drop the losing future. A future
there must be cancel-safe (`recv`, `accept`, `sleep`, `Notified`) or keep its
partial state outside the branch. `read_exact`, `write_all`, `read_line` into a
fresh buffer, or a half-consumed stream lose data when dropped: move them into
a task that feeds a channel, or loop on a cancel-safe step.

### Define admission and overload semantics

Every queue or asynchronous handoff must state:

- when the producer is acknowledged;
- what is durable;
- how retries and deduplication work;
- what happens at capacity;
- how shutdown drains.

Do not claim hard boundedness while allowing an unbounded ingress or an uncapped
priority class. Cap what is held, not only how many: queued bytes as well as
queue length.

### Bound every duration someone else controls

Clamp a `Retry-After`, cooldown, backoff, poll interval or lease that an
upstream, a user or a stored row supplies: a hub's `Retry-After: 349693` once
held six review Turns open for hours (#1443). A database call has a `statement_timeout`, and
a migration sets `lock_timeout` and finishes inside the backend's startup probe
window, or a rollout stalls the replica still serving.

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

Classify a failure by its variant, status or SQLSTATE, through one helper that
owns that classification (`is_statement_timeout`), never by a substring of its
message: `"500"` also matches an id. A `String` error on a domain API is a
defect.

### Log once, at the level the condition deserves

`ERROR` means a defect someone must fix, and it feeds the error-burst alert.
Log an expected outcome at `WARN` or below, with a counter: load shedding, a
throttle, a statement timeout, a cancellation, an upstream 5xx relayed to the
caller. A function that returns an error does not also log it; the code that
handles it logs once.

### Do not drop a result silently

`let _ =` on a fallible call, `.ok()` on a `Result`, and an ignored `send`
carry a `// reason:` naming why the loss is fine. A decode or parse result is
never passed through `unwrap_or_default`: it turns corrupt data into an empty
value no one sees.

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

### Control time in tests

A test waits on an event, not a clock: `#[tokio::test(start_paused = true)]` or
`tokio::time::pause` for timer logic, and a `Notify`, a channel or a gate file
for another task or process. A real `sleep` in a test is a flake source and
carries a `// reason:`.

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

Naming a `mod.rs` (or any file declaring `mod`) formats its whole subtree, not
just that file, and `--skip-children` is nightly-only. So `rustfmt <file>` does
not keep a diff narrow.

## Completion Check

Before declaring the Rust change complete:

- confirm every changed Rust path was checked against its applicable reference;
- confirm file contents match their assigned responsibility, including callers
  outside the owning directory;
- confirm critical task, queue, error, and conversion behavior is explicit where
  the change touches those boundaries, and that every duration and size from
  outside is bounded;
- run the crate-specific Clippy and test commands required by `AGENTS.md`;
- report any required check that could not be run.
