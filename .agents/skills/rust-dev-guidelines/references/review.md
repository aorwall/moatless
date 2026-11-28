# Reviewing Rust

The rule set a PR review loads for `*.rs` (see the `pr-review` skill's
"Loading the rules"). It lists what to check in a diff that CI cannot; the
rules themselves are in [`SKILL.md`](../SKILL.md) and the references it loads,
and a finding names the rule it breaks there.

## Leave to CI

Do not report what a CI job already fails: formatting, every lint in
`[workspace.lints]` and `clippy.toml` (`unwrap_used`, `print_*`,
`await_holding_lock`, the `disallowed_methods` fences such as
`reqwest::Client::new` and `unbounded_channel`), and the arch rules in
`tooling/arch-rules` (file size, roots, routes, SQL placement, public API,
single-implementation traits). Report a lint suppression only when its reason
is wrong.

## Check in the diff

Each check is `important` when the trigger it names is reachable, else a nit.

- **Cancellation.** For each new `.await` inside `select!`, `timeout` or a
  task that shutdown cancels: what is lost if the future is dropped there?
  A new `tokio::spawn` has an owner that sees it end; a new `spawn_blocking`
  can be stopped.
- **Durations and sizes from outside.** A sleep, retry, backoff, cooldown,
  lease or buffer sized from a header, a config value, a stored row or an
  upstream reply is clamped. A new SQL statement on a request path runs under a
  `statement_timeout`.
- **Dedup and idempotency keys.** A key that suppresses an echo, claims work or
  dedups a delivery names every input that changes the outcome: head SHA,
  session, turn, attempt. A retry after a partial write cannot apply twice.
- **State at the edges of a lifecycle.** What a close, restart, reconnect,
  token expiry or force-push does to pending items, cached credentials and
  in-flight turns.
- **Error semantics.** Variants a caller can act on, sources kept, a retryable
  failure not mapped to a permanent one, no substring classification, logged
  once at a level an alert can trust ("Log once, at the level the condition
  deserves").
- **Dropped results.** A new `let _ =`, `.ok()` or `unwrap_or_default` on a
  fallible or decode call carries its reason.
- **Defaults on the common path.** A new flag, field or config default is
  correct for the way most callers invoke it, not only for the case the PR was
  written for (`moat tasks create` defaults to `--headless true`).
- **SQL.** A listed projection, never `*`; a fetch cardinality that matches the
  query; a transaction around writes that must land together.
- **Tests.** The new test fails without the change and asserts behaviour, not
  a constant. It waits on an event, not a real `sleep`.
- **Doc comments.** A new item inserted between an existing doc comment and the
  item it documents moves that doc onto the wrong item: report it on the line.

## Migrations

`crates/core/migrations/**` is SQL, but its risk is the backend's: read the
LOOP's migrations row and check the cost of every statement on the largest
table it touches.
