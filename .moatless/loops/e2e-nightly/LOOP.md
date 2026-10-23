---
on: schedule
name: Nightly E2E
tags:
  - e2e-nightly
closeAfterSettledDays: 1
schedule:
  # 7 fields: sec min hour day-of-month month day-of-week year. Evaluated in
  # UTC by the runner, so `timezone` records the intent rather than shifting the
  # fire time — 03:00 UTC is the quiet hour for the cluster this runs against.
  cron: "0 0 3 * * * *"
  timezone: UTC
  message: "Run the API E2E suite against this cluster and report the result."
---

# Nightly E2E — the API suite against this cluster

Run the API end-to-end suite against the environment this task is itself running
in, and report what happened. The suite is what holds the knowledge about the
product; this prompt only says how to start it and what the report has to
contain.

Do not fix anything you find. A failure is the output of this run.

## Run it

The repository is already checked out. Work in the checkout, from its root.

```bash
bun install --frozen-lockfile
```

Sweep first. A run that was killed mid-flight never reached its own teardown, so
it left a task open — and an open task keeps a sandbox pod alive. Left there,
last night's leftovers read as tonight's failures.

```bash
E2E_PROFILE=k8s bun tests/sweep.ts --older-than 3h
```

Then the suite. The `k8s` profile points at the in-cluster backend and
authenticates as this sandbox's own user, so there is nothing to configure.

```bash
E2E_PROFILE=k8s bun run test:e2e
```

Expect about ten minutes — three chains at a time, most of it real sandbox pods
scheduling, pulling and starting. Do not pass `--reporter`; it would replace the
run-summary reporter whose output this report is built from.

If the suite fails to start because `moat api` is not a known command, the
`moat` on PATH is older than the suite's driver. Build the one in the checkout
and point the suite at it:

```bash
cargo build -p moatless-cli --bin moat
export MOAT_BIN="$PWD/target/debug/moat"
```

## Report it

Your final message is the report. It must carry, at minimum:

- The profile and base URL the run tested, and the run prefix — both are printed
  by `[global-setup]` at the top of the output and repeated by the run summary
  at the bottom.
- Counts of passed, failed and skipped.
- Every failure: its chain, its test, and the assertion that failed. "3 failed"
  costs whoever reads this a full re-run before they can act on it.
- Every skip, grouped by the reason the suite gave — each one names the profile
  capability that is missing. A tier that silently stopped running looks exactly
  like a healthy run if the skips are only counted.

If the run did not finish — the suite crashed, a command never returned, you ran
out of time — say that plainly and say where it stopped. An incomplete run
reported as a pass is worse than no run at all.

Finally, confirm the suite cleaned up after itself: teardown sweeps this run's
own prefix and prints what it removed. If anything failed to go, name it.
