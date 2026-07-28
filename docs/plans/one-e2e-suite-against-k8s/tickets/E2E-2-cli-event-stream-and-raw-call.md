# E2E-2 — CLI: stream the event feed, and a raw call that reports status

- **Priority:** P1
- **Status:** Done
- **Blocked by:** Nothing — runs in parallel with E2E-1
- **Spec:** [specs/one-e2e-suite-against-k8s.md](../../../../specs/one-e2e-suite-against-k8s.md)

## What to build

Two gaps in the moat CLI that the suite would otherwise reach around. Both are
useful outside the tests, and both belong in `crates/cli/src/commands/`.

**Stream the event feed.** `moat events` today only lists a task's history. The
suite needs to observe events as they arrive — the TypeScript suite maintains its
own collector for exactly this (`tests/shared/sse-collector.ts`). Add a streaming
subcommand that connects to the event stream and writes one JSON object per line
until interrupted, optionally filtered to a task. `MoatClient` already exposes a
streaming response helper (`crates/cli/src/client.rs:325`), so this is a command,
not a client change.

**A raw call that reports status.** The error-case tests assert on rejection
kinds — not found, forbidden, conflict — and the CLI currently maps failures to
messages and exit codes only. Add a raw request command taking a method, a path
and an optional body, printing the response status alongside the body as JSON,
and not failing the process on a non-success status. `raw_get`/`raw_post`/
`raw_put`/`raw_patch`/`raw_delete` already exist on the client; this exposes them.

Document both in the CLI help and in the `moat-cli` skill.

## Acceptance criteria

- [x] Streaming emits one JSON object per line and terminates cleanly on signal.
- [ ] Streaming works unchanged under sandbox auth and under an API key.
      *(Sandbox auth verified against the cluster. The API-key path needs
      seeded users, which only the `docker` profile has.)*
- [x] The raw command reports the status for both success and rejection.
- [x] A rejection does not make the process exit non-zero when raw is used.
- [x] Neither command changes the behaviour of an existing command.
- [x] Both appear in `--help` with an example.

## Verification

- Against Docker Compose: stream a task's events while it runs, confirm the
  lifecycle events arrive in order.
- Raw-call a nonexistent task and confirm the reported status is the rejection,
  and the exit code is zero.
- `cargo clippy -p moatless-cli -- -D warnings` and `cargo test -p moatless-cli`.
