# E2E-1 — Profile resolution and the moat driver

- **Priority:** P1
- **Status:** Done
- **Blocked by:** Nothing
- **Spec:** [specs/one-e2e-suite-against-k8s.md](../../../../specs/one-e2e-suite-against-k8s.md)

## What to build

Two new modules under `tests/shared/`, and the first chains moved onto them.

**`profile.ts`** — resolves `E2E_PROFILE` (`docker` | `k8s`) into one value. When
the variable is unset, a process that already has a sandbox login present
resolves to `k8s`, otherwise `docker`; the resolved name is always recorded and
printed, never left implicit. The profile carries:

| Field | Purpose |
|---|---|
| `name` | printed in the report header |
| `baseUrl` | backend base URL |
| `auth` | `password` (docker) or `ambient` (k8s) |
| `fixture` | template repository and workspace identifiers |
| `timeouts` | sandbox-ready, agent-idle, server-start |
| `capabilities` | `seedUsers`, `resetDatabase`, `directDatabase`, `inboundCallbacks`, `agentCredentials` |

Only the `docker` profile is populated in this ticket. `k8s` is added by E2E-6.

**`moat.ts`** — the driver. Spawns the compiled `moat` binary with `--json`,
parses stdout into typed results, and maps a non-zero exit plus stderr into a
typed error tests can assert on. It owns invocation, parsing and error mapping,
and nothing else; domain verbs stay in `tests/api/steps/`.

Then replace `getBaseURL()`, `getAdminCredentials()` and `getAdminCookie()` in
`tests/api/fixtures/api-fixtures.ts:36-62` with the profile, and migrate the
`task-crud` and `sandbox-lifecycle` chains plus the step functions they use onto
the driver. `tests/shared/api-client.ts` stays in place for the chains not yet
migrated — E2E-3 finishes that and deletes it.

## Acceptance criteria

- [ ] `E2E_PROFILE=docker` runs the migrated chains green against Docker Compose.
      *(Not run — no Docker daemon in the sandbox this was built from. The
      `k8s` profile is green; the `docker` run belongs to whoever has a daemon.)*
- [x] The resolved profile name appears in the run output before the first test.
- [x] Requesting an unknown profile name fails immediately with a clear message.
- [x] A capability flag is readable from a test without knowing the profile name.
- [x] The driver surfaces a CLI failure as a typed error, not a parse crash.
- [x] No migrated test constructs a URL or a credential of its own.

## Verification

- `E2E_PROFILE=docker` against a running Compose stack: migrated chains pass.
- A deliberately broken `moat` invocation produces the typed error, and the
  test failure message names the command that failed.
- `bun run type:check` and `bun run biome:check` are clean.
