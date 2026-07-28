# E2E-7 — The scheduled Loop that runs the suite nightly

- **Priority:** P2
- **Status:** Done
- **Blocked by:** E2E-6 (cluster profile green)
- **Spec:** [specs/one-e2e-suite-against-k8s.md](../../../../specs/one-e2e-suite-against-k8s.md)

## What to build

A Loop in this repository that runs the suite against the environment it is
itself running in, on a schedule.

Add `.moatless/loops/e2e-nightly/LOOP.md`, following the format the existing
`code-review` Loop uses: frontmatter declaring a schedule source with a cron
expression and a timezone, plus a body telling the agent what to do. The body is
short by design — check the repository out, run the suite under the cluster
profile, report the outcome — because the suite is what holds the knowledge, not
the prompt.

The reported summary must carry, at minimum: the profile, the run prefix, counts
of passed, failed and skipped, and for each failure the chain and the assertion.
A summary that says only "3 failed" costs a full re-run to act on.

Two operational details worth getting right in this ticket rather than after the
first bad night:

- The Loop sweeps stale resources before it starts, so a previous crashed run
  cannot make tonight's run look broken.
- The task the Loop creates is tagged so nightly runs are findable as a set.

## Acceptance criteria

- [x] The Loop definition is committed and picked up from the repository.
      *(Committed and parsed by the backend's own parser — see "What is not
      verified here" for the part that needs the default branch.)*
- [x] A manual trigger produces a complete run and a summary.
- [x] The summary names the profile, the counts, and each failure's chain.
- [x] Skips appear in the summary with their capability.
- [x] The run sweeps before starting.
- [x] A failing suite produces a summary that says so, rather than an incomplete run.

## Verification

- Trigger the Loop manually: it runs the suite and reports a summary.
- Break one test deliberately, trigger again, and confirm the failure is named in
  the summary.
- Confirm the created task carries the tag and leaves nothing behind.

## Closing notes

### The Loop

`.moatless/loops/e2e-nightly/LOOP.md`. Frontmatter is four things: `on: schedule`
with a 7-field cron at 03:00, the `e2e-nightly` tag so the runs are findable as a
set, and `inactiveTtlDays: 1` so a night's task closes itself rather than
accumulating. Everything else is default — `routingMode` stays `by_subject`, so
each fire is a new task rather than a conversation that grows forever, and
`agentType` stays `claude-code`.

The body is three commands and a report format. It says to sweep first, run the
suite under the `k8s` profile, and report; it deliberately does not explain what
the suite covers, because that is the suite's job and a prompt that repeats it
goes stale. It also says, in one line, not to fix anything — a nightly run that
starts editing the repository is worse than one that fails.

Two things in the body exist because of what the runs actually did:

- **`E2E_PROFILE=k8s` is written out** even though the profile would be inferred
  correctly inside a sandbox. A run that silently tested the wrong environment is
  the failure mode this whole plan exists to avoid.
- **"Do not pass `--reporter`"**, because the CLI flag replaces the config's
  reporters wholesale, and the run summary is what the report is built from. This
  cost an hour earlier in the plan; it is now written down where it is needed.

The cron is evaluated in **UTC**: `next_fire_at` parses the expression against
`Utc::now()` (`backend/src/adapters/common.rs`), and the `timezone` field is
carried into the schedule row but does not shift the fire time. `timezone: UTC`
in the frontmatter therefore states the truth rather than a preference.

### Product changes this ticket required

**The failure line in the run summary** (`tests/api/reporters/run-summary.ts`).
It printed the chain and the test title and nothing else, so the report a nightly
run could produce would have been "3 failed" — which the ticket calls out as
costing a full re-run to act on. It now prints the first line of the error, which
for this suite's assertions is the sentence the assertion was written to say:

```
Failed:
  Chain: Task CRUD › TASK-05: Set task tags
      the tag that was set is missing from the task
```

**Skips and not-reached are now separated.** Everything Playwright reports as
"skipped" used to land in one list. In a serial chain, the steps after a failure
are also skipped — with no annotation, because they were never started. Grouping
them with the capability skips made a chain that collapsed on its second step
read as an environment that cannot run it:

```
Did not run:
  7 × not reached — an earlier step in the chain failed
  39 × profile 'k8s' has no seedUsers
```

**The HTML reporter no longer opens on failure.** Playwright's default is to
serve the report and block until somebody closes it. Nobody is there: a Loop
would have hung immediately after the one outcome worth reporting. Set
`open: "never"` in `tests/api/playwright.config.ts`. This was found by running
the deliberately-broken suite, which hung exactly this way.

**A test that the repository's own loops parse**
(`backend/src/loops/sync/parser.rs::the_repositorys_own_loops_parse`). A
committed `LOOP.md` that does not parse is not an error anybody sees — sync logs
a warning and moves on, and the loop simply never fires. The test walks
`.moatless/loops/*/LOOP.md` with the backend's own parser, and for schedule loops
also parses the cron and asserts consecutive fires are at least an hour apart.
That last assertion is not decoration: earlier in this task a schedule loop with
too fast a cron produced about fifty runaway tasks before it was deactivated.
Verified non-vacuous by temporarily setting the cron to `0 * * * * * *`, which
fails it.

### The runs

**A complete run and its summary** — the loop body's own commands, executed in
this sandbox, which is the environment a nightly task runs in:

```
Run e2e-ms3e8ldv-ge1x — profile 'k8s' → http://moatless-vibe-backend.moatless.svc.cluster.local:8080
  (without seedUsers, resetDatabase, directDatabase, inboundCallbacks)
150 passed, 0 failed, 67 skipped (passed)
```

Wall clock 15:40:28 → 15:47:25 UTC — seven minutes, at the default three
workers. The skip breakdown the summary printed:

| Count | Reason |
|---|---|
| 39 | `profile 'k8s' has no seedUsers` |
| 8 | `profile 'k8s' has no directDatabase` |
| 7 | `GITHUB_PAT is not set` |
| 7 | `profile 'k8s' has no seedUsers, directDatabase` |
| 5 | `profile 'k8s' has no inboundCallbacks` |
| 1 | `Needs a locally built image, which only the Docker profile can schedule` |

**A failing run** — `TASK-05` broken deliberately (`toContain("deliberate-break")`
with the assertion message above). The summary named the chain, the test and the
assertion, listed the seven steps that never ran with the reason, and the process
exited instead of serving a report. Restored afterwards; `git diff` on the spec
is empty.

**Sweeping and leftovers.** Teardown swept the run's own prefix and reported one
failure, honestly:

```
[sweep] removed nothing for e2e-ms3e8ldv-ge1x, 1 failed — this environment's backend has no
DELETE /api/v1/adapters/connections/{id} (HTTP 404) — connections will accumulate until it
is redeployed
```

That is the endpoint added in E2E-5's closing work; the cluster's backend
predates it. It resolves on the next deploy, and the sweep says so rather than
failing silently.

### What is not verified here

**The Loop has not fired on the cluster.** Loop sync reads
`.moatless/loops/` through the GitHub contents API with no `ref`, which is the
repository's **default branch**. This work is on `e2e/unified-suite`, so the
trigger row cannot exist until it merges. Everything before that point is
verified — the file parses with the backend's parser, resolves to a schedule
source, and the body's commands were run end to end — but three things need one
look after the merge:

1. `moat loops list --kind schedule` shows `e2e-nightly` against this repository.
2. The first fire creates a task carrying the `e2e-nightly` tag.
3. That task's report matches the shape above, and it leaves nothing behind.

Deliberately not done: creating an equivalent schedule loop by hand to force a
fire early. It would have run against `main`, where the suite does not exist yet,
so it would have proved nothing about the suite and everything about a task that
cannot find its tests — while spending a sandbox and an agent to do it.
