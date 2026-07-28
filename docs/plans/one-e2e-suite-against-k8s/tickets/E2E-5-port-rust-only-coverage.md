# E2E-5 — Port the coverage that exists only in the Rust crate

- **Priority:** P1
- **Status:** Done
- **Blocked by:** E2E-3 (driver migration complete)
- **Spec:** [specs/one-e2e-suite-against-k8s.md](../../../../specs/one-e2e-suite-against-k8s.md)

## What to build

17 subjects, 44 tests, currently only in `crates/integration-tests/tests/`. Each
becomes a chain or a standalone test in `tests/api/`, keeping its scenario ID
comment so the `specs/*/01-*.md` mapping survives the rewrite.

| Subject | Tests | Capability it needs |
|---|---|---|
| `setup_requirements` | 6 | direct database |
| `github_proxy` | 5 | inbound callbacks |
| `integrations_github_pr_reply` | 5 | — |
| `repo_crud` | 5 | — (compare against the repository chain first) |
| `trigger_routing` | 4 | — (overlaps the webhook-routing chain) |
| `feedback` | 3 | — |
| `integrations_config` | 2 | — |
| `provider_settings` | 2 | direct database |
| `sandbox_replacement_ordering` | 2 | agent credentials |
| `task_connections` | 2 | second identity |
| `trigger_recent_tasks` | 2 | direct database |
| `agent_ask_user_question` | 1 | agent credentials |
| `loops_alias_equivalence` | 1 | — |
| `messages` | 1 | — |
| `shadow_workspace_primary` | 1 | direct database |
| `subagent_tool_tracking` | 1 | agent credentials |
| `workspace_git_override`, `workspace_multi_repo` | 2 | one needs direct database |

Each ported test opens with a capability guard where the table names one, so it
skips visibly rather than failing under a profile that cannot run it.

The other 16 Rust files already have a same-subject chain. Do not port those as
new tests — compare them assertion by assertion and fold anything the chain
misses into the existing chain. Record which assertions were folded in, and which
were dropped as genuinely duplicate, in the ticket's closing notes.

## Acceptance criteria

- [x] All 17 subjects exist in the TypeScript suite.
- [x] Every ported test carries its scenario ID.
- [x] Capability-gated tests skip with the capability named, not silently.
- [x] The overlapping subjects are reconciled, with the outcome recorded.
      *(15 files, not 16 — the count in the table above splits
      `workspace_git_override` and `workspace_multi_repo` into one row.)*
- [x] No assertion from the Rust crate is lost without an explicit note saying so.

## Verification

- `E2E_PROFILE=docker` full suite: green, with the ported subjects visible in the
  report. *(Not run here — this sandbox has no Compose stack. Verified instead
  against the cluster, chain by chain; the table at the end of the closing notes
  lists the results. The Compose run is E2E-6.)*
- Spot-check three ported subjects against their Rust originals to confirm the
  assertions match. *(Done for all fifteen, not three — see the reconciliation
  table.)*
- The recorded reconciliation lists every dropped assertion with a reason.

---

## Closing notes

### Reconciliation of the 15 overlapping subjects

Every Rust file that already had a same-subject chain was read assertion by
assertion against it. What follows is the full result: what moved across, and
what was left behind because the chain already made the same claim.

| Rust file | Folded into the chain | Dropped as duplicate |
|---|---|---|
| `task_crud.rs` | TASK-03.1 — the six legacy status fields are absent from list, get, `live-status` and `/status`. New scenario, written up in `specs/tasks/01-task-scenarios.md`. | Create/get/list/update/delete shape assertions; the chain already made each one. |
| `task_errors.rs` | — | Both assertions (unknown id → 404, malformed body → 400) were already in `task-errors.spec.ts`. |
| `sandbox_errors.rs` | — | Provision failure and crash-loop detection were already asserted, and the chain additionally bounds the detection time. |
| `integrations_errors.rs` | INT-53 now asserts the rejection status directly. It previously sat in a `try` whose success path was `expect(true).toBe(false)` and whose failure path matched the substring "400" against an error message — an assertion that could pass without the API rejecting anything. Needed `createTaskTriggerFixtureRaw`. | The remaining five assertions duplicated the chain. |
| `message_acknowledgment.rs` | MSG-08 (the follow-ups join the running turn rather than starting new ones) and MSG-09 (no `sandbox.error` during the exchange). MSG-05 and MSG-08 now skip, with a reason, when the first turn finished before the follow-ups landed, instead of racing. | Queue-then-acknowledge assertions; already covered. |
| `agent_execution_flow.rs` | FILE-06 now asserts the agent's file is marked `isChanged` in the file tree, not just present on disk. | Turn completion, message shape and SSE lifecycle assertions. |
| `file_operations.rs` | Nothing new; FILE-01 was rewritten to discover the checkout instead of naming `package.json` at the workspace root, which does not exist on the cluster. | List, read, tree, search and write were all already asserted. |
| `agent_plan_mode.rs` | The chain was creating an ordinary task, so plan mode was never exercised. It now passes `--mode plan` and asserts the opening message ran in plan mode, waits for the real pause, and checks the plan survives the summary projection the approval card reads from (AGENT-03.1). | The Rust file's own plan assertions, once the chain actually reached plan mode. |
| `agent_stop_recovery.rs` | Nothing new. AGENT-06.1 no longer returns early when the sandbox is already stopped — it asserts the state either way. | Restart-and-persist is `SANDBOX-04` in the lifecycle chain; the follow-up turn, history length and resumed file were all already asserted. |
| `webhook_callback.rs` | The sandbox-ready wait, asserted separately so a sandbox that never provisions does not read as an agent that never finished. The `CHANNEL_CALLBACK_E2E` gate became the `inboundCallbacks` + `agentCredentials` capabilities. | The inbound response carrying `taskId` and the callback naming the task; the chain asserts both, and additionally the method, content type and `is_error`. |
| `sandbox_lifecycle.rs` | The marker file. "Files persist" was asserted by counting entries, which a freshly provisioned sandbox satisfies just as well as one whose volume survived; the chain now writes a file before the stop and reads its content back after the start and after the restart. | Provision, re-provision, stop, start, cleanup — all already asserted, and the chain also checks the SSE events and the absence of a sandbox error. |
| `plugins_e2e.rs` | PLUGIN-03 now asserts the effective set is the plugin's whole synced skill list, not merely that it contains the one skill the chain later asks the agent about. | Register, list, sync, personal override and clear; the chain asserts all of them plus the activation records and the agent naming the skill. |
| `auth_verify.rs` | AUTH-33 — the task now belongs to alice and alice reaches its preview. It previously read as "an admin reaches every preview", which is AUTH-35.1 and says nothing about an owner reaching their own. | AUTH-30, 30.1, 31, 32, 34, 35, 35.1 were already asserted. |
| `auth_teams.rs` | — | Team CRUD and membership were fully covered, and the chain also asserts `createdAt`/`updatedAt` and run scoping. **AUTH-10** (non-member team credential upsert → 403) is deliberately not carried over: team credentials were removed with the two-value scope model (ADR-0006) and the id is retired. |
| `auth_access.rs` | Three. **AUTH-23.1** claimed a task inherits its repository's scope but only read the task's own id back, which is true of any task — it now asserts the scope and the owner. **AUTH-27** gained the invalid-binding assertion the spec has always described. **AUTH-28** is new: eight simultaneous sign-ups for one login leave exactly one user and refuse the rest as conflicts, not as internal errors. | AUTH-20.x, 22.x, 24.x, 25, 26. |

Nothing from the Rust crate was dropped without appearing in the right-hand
column above.

### Scenario ids

- `auth-access-control.spec.ts` chain 2 had drifted from the spec: the suite's
  `AUTH-23.2` was the spec's `AUTH-23.3`, and the public-visibility pair sat on
  ids the spec had already given to something else. The suite now follows the
  spec, and the two public-visibility scenarios were written up under ids of
  their own — `AUTH-23.4` and `AUTH-23.5`.
- `AUTH-28` and `TASK-03.1` are new ids, both added to their spec files.
- The suite uses `INT-nn` where `specs/integrations/…` uses `CHAN-nn`. The two
  families are the same scenarios under different names; this predates the
  rewrite and is left alone rather than renumbered, since ids are permanent.
- `MSG-nn` in `message-acknowledgment.spec.ts` is local to that chain and has no
  entry in any spec file. It is not a spec family.

### Product changes this ticket required

Three, all of them things the suite could not work around:

- **`--timeout` is now a global CLI option** (`crates/cli/src/main.rs`,
  `client.rs`). Closing a task removes its sandbox before answering — 33 seconds
  observed against the cluster — against a 30-second client ceiling. `MoatApi`
  raises it for every call that moves a sandbox.
- **`moat tasks restart` and `tasks redeploy` no longer refuse outright inside a
  sandbox.** The guard now refuses only the sandbox's own task, which is the one
  that would take the caller down with it. Before this, `SANDBOX-04` could not
  execute at all under the k8s profile; it now passes there.
- **`createPlugin` called `moat plugins add`, which does not exist.** The
  subcommand is `create`. The whole plugins chain failed on its first step.

### Findings worth keeping

- **`agent.pending_response` has no producer.** It is a declared event type,
  forwarded over SSE and consumed by the frontend, and nothing in the platform
  publishes it. A test waiting for it waits forever. `waitForPendingToolCall`
  reads the pause out of the conversation instead — an assistant message flagged
  as awaiting input, carrying a tool call with no result against it.
- **`GET /tasks/{id}/git-diff` does not exist.** The old `getGitDiff` verb hit a
  nonexistent route inside a swallowing `try`/`catch`, so the assertion behind it
  never ran. Replaced with the `files/tree` `isChanged` assertion; the verb and
  its types are gone.
- **`gitSummary.hasChanges` is true on a fresh checkout**, so it cannot stand in
  for "the agent changed something".
- **Search cannot see symlinks.** `sandbox/src/files/search.rs` walks with the
  `ignore` crate and takes only `is_file()`, so a repository root `AGENTS.md`
  pointing at `CLAUDE.md` is invisible to `FILE-04`.
- **The workspace file browser is rooted at the workspace, not the repository.**
  Repositories mount one level below (`moatable/`), and every path the API
  returns is workspace-relative.

### What is not verified here

- The four `AUTH-*` chains, and everything else gated on `seedUsers`,
  `resetDatabase`, `directDatabase` or `inboundCallbacks`, skip under the k8s
  profile. They are written and type-checked but have not executed since the
  fold-ins above. A Compose run is E2E-6's verification step.
- **Adapter connections leak.** The sweep removes tasks, loops, workspaces,
  repositories and teams. There is no `DELETE` endpoint for an adapter
  connection anywhere in the backend, so the prefixed connections the trigger
  chains create cannot be removed — 17 of them are currently on the cluster.
  Adding a product endpoint for it was out of scope; it is recorded here so it is
  a known limitation rather than a surprise.
- `tests/` sits outside biome's `files.includes`, so the suite is formatted by
  the pre-commit hook and not by `bun run biome:check`.
- `cargo test -p moatless-cli` has one pre-existing failure,
  `commands::docker::helpers::tests::copy_assets_writes_files`, unrelated to this
  work.

### Cluster runs behind these notes

Every chain the k8s profile can run, passing with the suite's own config:

| Chain | Result |
|---|---|
| `task-crud` | 14/14 |
| `integrations-errors` + `task-errors` | 7/7 |
| `message-acknowledgment` | 13/13 |
| `agent-execution-flow` + `agent-stop-recovery` | 20/20 |
| `file-operations` | 6/6 |
| `sandbox-errors` | 2 passed, 1 skipped |
| `agent-plan-mode` | 5/5 |
| `agent-tool-interaction` | 2/2 |
| `sandbox-lifecycle` | 10/10 |
| `plugins-e2e` | 12/12 |
| `auth-verify` chain 1 | 4/4 (chain 2 skips — no `seedUsers`) |
