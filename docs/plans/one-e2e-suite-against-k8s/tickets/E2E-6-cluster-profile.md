# E2E-6 — The cluster profile

- **Priority:** P1
- **Status:** Done
- **Blocked by:** E2E-4 (isolation), E2E-5 (coverage ported)
- **Spec:** [specs/one-e2e-suite-against-k8s.md](../../../../specs/one-e2e-suite-against-k8s.md)

## What to build

Populate the `k8s` profile and make the suite run green against the deployed
environment from inside a sandbox.

**Authentication** is ambient: a process inside a sandbox is already logged in as
the task's owner, so the profile's auth strategy is "use what the environment
gives you" and there is no credential to store anywhere.

**Fixture** is the Moatable template repository and its workspace — a React and
Vite starter with a preview server, the cluster's equivalent of the template the
Docker profile uses. The profile resolves it by identifier and fails fast with a
clear message if it is missing, rather than creating one.

**Capabilities** under `k8s`: second identity off, database reset off, direct
database access off, inbound callbacks off. Every test gated on those skips, and
the report lists each skip with the capability that caused it — a skipped test
must be as visible as a failing one, or coverage quietly erodes.

**Timeouts** are pod-speed, not container-speed: sandbox provisioning, agent
idle and preview-server start all need longer ceilings than the Docker profile.
Derive them from observed cluster behaviour, and state the observed values in the
profile so a future change has a baseline to argue with.

## Acceptance criteria

- [x] `E2E_PROFILE=k8s` runs from inside a sandbox with no extra configuration.
      *(One caveat today: the `moat` on a sandbox's PATH predates `moat api`,
      which E2E-2 added, so a run currently needs `MOAT_BIN` pointed at a
      locally built CLI. It resolves itself when the sandbox image next ships.)*
- [x] All three tiers run — including the agent tier.
- [x] Skips are listed in the report with their capability, and do not fail the run.
- [x] A missing fixture repository fails immediately with an actionable message.
- [x] The run creates only prefixed resources and removes them at the end.
- [x] Nothing in the suite writes to the cluster database directly.

## Verification

- Run from a sandbox against the deployed environment: green, with the expected
  set of skips and nothing left behind afterwards.
- Check the environment for leftovers after a completed run: none.
- Kill a run mid-chain, sweep, and confirm the orphaned sandbox is gone.

---

## Closing notes

### The run

`E2E_PROFILE=k8s`, from inside a sandbox, against the deployed environment:

```
150 passed, 0 failed, 67 skipped
```

All three tiers, agent included — the agent-tier chains reach a real model
through the cluster's own credentials, which is why `agentCredentials` is the
one capability the k8s profile has and the Docker profile has to earn from the
environment.

### Observed timings, which is where the ceilings come from

| Transition | Observed | Ceiling |
|---|---|---|
| Sandbox reaches ready | 70–80s typical, ~40s on a restart | 420s |
| Agent returns to idle | 30–90s for a single-file turn | 420s |
| Preview server serves | not exercised by a green chain | 180s |

The ceilings sit far above the observations on purpose: a pod that has to
schedule, pull and mount is not bounded by its median. `waitForSandboxReady`
and the sandbox steps now read their budget from the profile rather than a
constant — they were hardcoded at 180s, which is under what the k8s profile
asks for.

### Skips, and what causes each

```
39 × profile 'k8s' has no seedUsers
 8 × profile 'k8s' has no directDatabase
 7 × GITHUB_PAT is not set
 7 × profile 'k8s' has no seedUsers, directDatabase
 5 × profile 'k8s' has no inboundCallbacks
 1 × Needs a locally built image, which only the Docker profile can schedule
```

Printed by `tests/api/reporters/run-summary.ts`, which groups the skips by their
reason and lists the tests under each. The built-in reporters count skips, and a
count reads the same whether the environment lacks a capability or a whole tier
quietly stopped executing.

### Fixture

Checked once, in global setup, before any chain starts:

```
[global-setup] Fixture repository: Moatable (433e035b-f37d-4992-834a-01ed779996e7)
```

A missing one stops the run there, naming the identifier and the variable that
overrides it, rather than failing every chain in turn on the same thing. A
driver that could not start at all is reported as itself — that distinction was
worth making, because an unreadable `MOAT_BIN` had been surfacing as "the
fixture repository is missing", which sends the reader somewhere useless.

### Leftovers

Nothing after a completed run: 0 prefixed tasks, repositories, workspaces or
teams.

Adapter connections were the exception and are now fixed at the source. The
suite creates one per trigger fixture — fourteen per full run — and there was no
`DELETE` route for them, though the repository has had a scope-filtered `delete`
all along. The route now exists and the sweep uses it. Against a backend older
than the route the sweep says so once and stops:

```
[sweep] removed nothing for every run, 1 failed — this environment's backend has no
DELETE /api/v1/adapters/connections/{id} (HTTP 404) — connections will accumulate
until it is redeployed
```

The 31 connections currently on the cluster clear on the first sweep after the
backend ships.

### Killed run, swept

Run killed at `SANDBOX-03.1`, its sandbox restarted by hand to `ready` so the
sweep had a live pod to deal with rather than a stopped one:

```
[sweep] removing runs older than 0 minutes
removing task e2e-ms3cv2op-dh7i-task
```

The task reads 404 afterwards and the sandbox is gone with it.
